//! The WebSocket open/read check that decides `online`.

use std::fmt;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;

use futures::{SinkExt, StreamExt};
use nostr::event::Kind;
use nostr::filter::Filter;
use nostr::message::{ClientMessage, RelayMessage, SubscriptionId};
use relays_core::observe::{Check, Fault, Obstacle, Outcome};
use relays_core::url::RelayUrl;
use rustls::pki_types::ServerName;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::time::{Instant, timeout, timeout_at};
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;
use tokio_tungstenite::tungstenite::client::ClientRequestBuilder;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tokio_tungstenite::{WebSocketStream, client_async_with_config};
use tracing::debug;

use super::resolve::{self, DnsError};
use super::{
    CLOSE_TIMEOUT, CONNECT_ATTEMPT_TIMEOUT, MAX_MESSAGE, OPEN_TIMEOUT, Prober, READ_TIMEOUT,
    USER_AGENT,
};
use crate::now_unix;

/// Errors during the open phase; translated into outcome fields.
#[derive(Debug)]
enum OpenError {
    /// The open deadline elapsed before the 101 response.
    Timeout,
    /// TLS handshake failure reported by rustls.
    Tls(io::Error),
    /// The upgrade got a non-101 response; carries the status and `Location`.
    Http(u16, Option<String>),
    /// Protocol violation during the handshake.
    Protocol(String),
    /// Any other IO error at the TCP, TLS or upgrade level.
    Io(io::Error),
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timeout => f.write_str("open deadline elapsed"),
            Self::Tls(e) | Self::Io(e) => write!(f, "{e}"),
            Self::Http(status, _) => write!(f, "HTTP {status}"),
            Self::Protocol(e) => f.write_str(e),
        }
    }
}

/// Run one check: DNS, address policy, TCP + optional TLS + WebSocket
/// upgrade, then the read check (one REQ) and a closing handshake.
pub(crate) async fn check(prober: &Prober, url: &RelayUrl) -> Check {
    let started = Instant::now();
    let mut check = Check {
        at: now_unix(),
        outcome: Outcome::Online,
        http_status: None,
        location: None,
        rtt_open: None,
        rtt_read: None,
        auth: false,
    };
    let ips = match resolve::lookup(&prober.resolver, url.host(), url.ip()).await {
        Ok(ips) => ips,
        Err(e) => {
            check.outcome = match e {
                DnsError::Nxdomain => Outcome::Offline(Fault::DnsNxdomain),
                DnsError::Transient(_) => Outcome::Unverified(Obstacle::DnsTransient),
            };
            debug!(url = %url, reason = check.outcome.reason().unwrap_or(""), error = %e, "check failed");
            return check;
        }
    };
    if let Some(ip) = ips.iter().find(|ip| !prober.policy.allows(**ip)) {
        check.outcome = Outcome::Offline(Fault::PrivateAddress);
        debug!(url = %url, reason = check.outcome.reason().unwrap_or(""), error = %ip, "check failed");
        return check;
    }
    let addrs: Vec<SocketAddr> = ips
        .iter()
        .map(|ip| SocketAddr::new(*ip, url.port()))
        .collect();
    let deadline = started + OPEN_TIMEOUT;
    if let Err(e) = open(prober, url, &addrs, deadline, started, &mut check).await {
        classify_open(&e, &mut check);
        debug!(url = %url, reason = check.outcome.reason().unwrap_or(""), error = %e, "check failed");
    }
    check
}

fn classify_open(error: &OpenError, check: &mut Check) {
    check.outcome = match error {
        OpenError::Timeout => Outcome::Offline(Fault::Timeout),
        OpenError::Tls(_) => Outcome::Offline(Fault::Tls),
        OpenError::Io(e) => Outcome::Offline(classify_io(e)),
        OpenError::Protocol(_) => Outcome::Offline(Fault::Protocol),
        OpenError::Http(status, location) => {
            check.http_status = Some(*status);
            check.location.clone_from(location);
            match status {
                300..=399 => Outcome::Offline(Fault::Moved),
                401 | 403 | 429 => Outcome::Unverified(Obstacle::Blocked),
                500..=599 => Outcome::Offline(Fault::ServerError),
                _ => Outcome::Offline(Fault::NotWebsocket),
            }
        }
    };
}

/// Open phase: connect to each resolved address in order, then upgrade.
/// Every step shares `deadline`, which covers DNS through the 101 response;
/// the read check runs after the upgrade with its own budget.
async fn open(
    prober: &Prober,
    url: &RelayUrl,
    addrs: &[SocketAddr],
    deadline: Instant,
    started: Instant,
    check: &mut Check,
) -> Result<(), OpenError> {
    let tcp = connect(addrs, deadline).await.map_err(|e| {
        if e.kind() == io::ErrorKind::TimedOut {
            OpenError::Timeout
        } else {
            OpenError::Io(e)
        }
    })?;
    drop(tcp.set_nodelay(true));
    if url.is_secure() {
        let stream = timeout_at(deadline, tls_connect(prober, url, tcp))
            .await
            .map_err(|_| OpenError::Timeout)??;
        upgrade_and_read(url, stream, deadline, started, check).await
    } else {
        upgrade_and_read(url, tcp, deadline, started, check).await
    }
}

/// Connect to each address in resolver order until one succeeds. Every
/// attempt is capped at [`CONNECT_ATTEMPT_TIMEOUT`] and the whole loop by
/// `deadline`; returns the last error when all fail.
///
/// # Errors
///
/// Returns the last connection error, [`io::ErrorKind::TimedOut`] when an
/// attempt or the deadline elapsed, or [`io::ErrorKind::AddrNotAvailable`]
/// when `addrs` is empty.
pub async fn connect(addrs: &[SocketAddr], deadline: Instant) -> io::Result<TcpStream> {
    let mut last = io::Error::new(io::ErrorKind::AddrNotAvailable, "no resolved addresses");
    for addr in addrs {
        let cap = deadline.min(Instant::now() + CONNECT_ATTEMPT_TIMEOUT);
        match timeout_at(cap, TcpStream::connect(*addr)).await {
            Ok(Ok(stream)) => return Ok(stream),
            Ok(Err(e)) => last = e,
            Err(_) => {
                last = io::Error::new(io::ErrorKind::TimedOut, "connect attempt timed out");
            }
        }
    }
    Err(last)
}

/// TLS handshake over `tcp`. Only errors reported by rustls become
/// [`OpenError::Tls`]; other IO failures stay [`OpenError::Io`].
async fn tls_connect(
    prober: &Prober,
    url: &RelayUrl,
    tcp: TcpStream,
) -> Result<TlsStream<TcpStream>, OpenError> {
    let server = match url.ip() {
        Some(ip) => ServerName::IpAddress(ip.into()),
        None => ServerName::try_from(url.host().to_owned())
            .map_err(|e| OpenError::Protocol(e.to_string()))?,
    };
    TlsConnector::from(Arc::clone(&prober.tls))
        .connect(server, tcp)
        .await
        .map_err(|e| {
            if e.get_ref()
                .is_some_and(<dyn std::error::Error + Send + Sync>::is::<rustls::Error>)
            {
                OpenError::Tls(e)
            } else {
                OpenError::Io(e)
            }
        })
}

async fn upgrade_and_read<S>(
    url: &RelayUrl,
    stream: S,
    deadline: Instant,
    started: Instant,
    check: &mut Check,
) -> Result<(), OpenError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut ws = timeout_at(deadline, upgrade(url, stream))
        .await
        .map_err(|_| OpenError::Timeout)??;
    check.rtt_open = Some(millis(started.elapsed()));
    read_check(&mut ws, check).await;
    Ok(())
}

async fn upgrade<S>(url: &RelayUrl, stream: S) -> Result<WebSocketStream<S>, OpenError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let request = ClientRequestBuilder::new(
        url.as_str()
            .parse()
            .map_err(|e| OpenError::Protocol(format!("{e}")))?,
    )
    .with_header("User-Agent", USER_AGENT);
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE))
        .max_frame_size(Some(MAX_MESSAGE));
    let (ws, _response) = client_async_with_config(request, stream, Some(config))
        .await
        .map_err(classify_ws)?;
    Ok(ws)
}

fn classify_io(error: &io::Error) -> Fault {
    match error.kind() {
        io::ErrorKind::ConnectionRefused
        | io::ErrorKind::ConnectionReset
        | io::ErrorKind::ConnectionAborted => Fault::Refused,
        _ => Fault::Network,
    }
}

fn classify_ws(error: WsError) -> OpenError {
    match error {
        WsError::Http(response) => {
            let status = response.status().as_u16();
            let location = response
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned);
            OpenError::Http(status, location)
        }
        WsError::Io(e) => OpenError::Io(e),
        WsError::Protocol(_) | WsError::Capacity(_) | WsError::Utf8(_) | WsError::HttpFormat(_) => {
            OpenError::Protocol(error.to_string())
        }
        _ => OpenError::Io(io::Error::other(error.to_string())),
    }
}

async fn read_check<S>(ws: &mut WebSocketStream<S>, check: &mut Check)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let subscription = SubscriptionId::generate();
    let req = ClientMessage::req(
        subscription.clone(),
        Filter::new().kind(Kind::TextNote).limit(1),
    )
    .as_json();
    let sent = Instant::now();
    if ws.send(Message::text(req)).await.is_ok() {
        await_response(ws, sent, check).await;
    }
    let close = ClientMessage::close(subscription).as_json();
    let _closing = timeout(CLOSE_TIMEOUT, async {
        drop(ws.send(Message::text(close)).await);
        drop(ws.close(None).await);
    })
    .await;
}

async fn await_response<S>(ws: &mut WebSocketStream<S>, sent: Instant, check: &mut Check)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    drop(
        timeout(READ_TIMEOUT, async {
            while let Some(frame) = ws.next().await {
                let text = match frame {
                    Ok(Message::Text(text)) => text,
                    Ok(_) => continue,
                    Err(_) => break,
                };
                match RelayMessage::from_json(text.as_str()) {
                    Ok(RelayMessage::Auth { .. }) => check.auth = true,
                    Ok(RelayMessage::Event { .. } | RelayMessage::EndOfStoredEvents(_)) => {
                        check.rtt_read = Some(millis(sent.elapsed()));
                        return;
                    }
                    Ok(RelayMessage::Closed { message, .. }) => {
                        check.auth |= message.starts_with("auth-required:");
                        return;
                    }
                    _ => {}
                }
            }
        })
        .await,
    );
}

fn millis(duration: std::time::Duration) -> u32 {
    u32::try_from(duration.as_millis()).unwrap_or(u32::MAX)
}
