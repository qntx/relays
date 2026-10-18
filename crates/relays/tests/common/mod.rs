//! In-process mock relay used by the probe and end-to-end tests.

#![allow(
    dead_code,
    reason = "variants are used selectively by each test binary"
)]

use futures::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;

/// What the mock relay sends after the client's REQ.
#[derive(Debug, Clone, Copy)]
pub(crate) enum RelayReply {
    /// `["EOSE", "<sub>"]`.
    Eose,
    /// `["AUTH", "challenge"]` then `["EOSE", "<sub>"]`.
    AuthChallenge,
    /// `["CLOSED", "<sub>", "auth-required: no pubkey"]`.
    ClosedAuthRequired,
    /// Nothing; used to exercise the read timeout.
    Silent,
}

/// What the mock server does with an incoming connection.
#[derive(Debug)]
#[allow(
    variant_size_differences,
    reason = "the Http variant carries a canned response; size is irrelevant in tests"
)]
pub(crate) enum Mode {
    /// Reply with a fixed HTTP response and close.
    Http(&'static str),
    /// Complete the WebSocket upgrade and reply per [`RelayReply`].
    WebSocket(RelayReply),
    /// Delay the upgrade response, then behave like [`Mode::WebSocket`].
    Delayed(std::time::Duration, RelayReply),
    /// Like `WebSocket(Eose)` but reports whether the client sent a
    /// `["CLOSE", <sub>]` message and a WebSocket close frame.
    WebSocketClose(tokio::sync::oneshot::Sender<(bool, bool)>),
    /// Write bytes that are not a TLS record, for handshake classification.
    Garbage,
    /// Accept the connection and close it immediately.
    Close,
    /// Accept the connection and never speak (open timeout).
    Dead,
}

/// Spawn a mock relay on a loopback port; returns its canonical URL.
pub(crate) async fn spawn(mode: Mode) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        match mode {
            Mode::Http(response) => {
                read_request(&mut stream).await;
                stream.write_all(response.as_bytes()).await.unwrap();
            }
            Mode::WebSocket(reply) => {
                let mut ws = accept_async(stream).await.unwrap();
                answer_req(&mut ws, reply).await;
            }
            Mode::Delayed(delay, reply) => {
                // The client's upgrade request waits in the socket buffer
                // while the delay runs, so the 101 lands `delay` later.
                tokio::time::sleep(delay).await;
                let mut ws = accept_async(stream).await.unwrap();
                answer_req(&mut ws, reply).await;
            }
            Mode::WebSocketClose(report) => {
                let mut ws = accept_async(stream).await.unwrap();
                capture_teardown(&mut ws, report).await;
            }
            Mode::Garbage => {
                // Let the client send its ClientHello, then reply with bytes
                // that are not a TLS record.
                let mut buf = [0_u8; 512];
                drop(stream.read(&mut buf).await);
                stream.write_all(b"this is not a TLS record").await.unwrap();
            }
            Mode::Close => drop(stream),
            Mode::Dead => {
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            }
        }
    });
    format!("ws://127.0.0.1:{port}/")
}

/// Read the client's REQ, reply per `reply`, then keep the socket open so
/// the client observes silence (or the close handshake).
async fn answer_req<S>(ws: &mut tokio_tungstenite::WebSocketStream<S>, reply: RelayReply)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    // Read the REQ before replying.
    drop(ws.next().await);
    match reply {
        RelayReply::Eose => send(ws, r#"["EOSE","sub"]"#).await,
        RelayReply::AuthChallenge => {
            send(ws, r#"["AUTH","challenge"]"#).await;
            send(ws, r#"["EOSE","sub"]"#).await;
        }
        RelayReply::ClosedAuthRequired => {
            send(ws, r#"["CLOSED","sub","auth-required: no pubkey"]"#).await;
        }
        RelayReply::Silent => {}
    }
    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
}

async fn send<S>(ws: &mut tokio_tungstenite::WebSocketStream<S>, text: &str)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    ws.send(Message::text(text)).await.unwrap();
}

async fn read_request(stream: &mut tokio::net::TcpStream) {
    let mut buf = vec![0_u8; 4096];
    let mut got = 0;
    while got < buf.len() {
        let n = stream.read(&mut buf[got..]).await.unwrap_or(0);
        if n == 0 {
            break;
        }
        got += n;
        if buf[..got].windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
}

/// Reply with EOSE, then report whether the client closed with a
/// `["CLOSE", <sub>]` message and a WebSocket close frame.
async fn capture_teardown<S>(
    ws: &mut tokio_tungstenite::WebSocketStream<S>,
    report: tokio::sync::oneshot::Sender<(bool, bool)>,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    // First frame is the REQ; remember its subscription id.
    let sub = match ws.next().await {
        Some(Ok(Message::Text(text))) => serde_json::from_str::<serde_json::Value>(text.as_str())
            .ok()
            .and_then(|v| v.get(1).and_then(|i| i.as_str().map(str::to_owned)))
            .unwrap_or_default(),
        _ => String::new(),
    };
    send(ws, r#"["EOSE","sub"]"#).await;
    let mut close_msg = false;
    let mut close_frame = false;
    let _drained = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while let Some(frame) = ws.next().await {
            match frame {
                Ok(Message::Text(text)) => close_msg |= is_close_for(&text, &sub),
                Ok(Message::Close(_)) => close_frame = true,
                _ => {}
            }
        }
    })
    .await;
    let _sent = report.send((close_msg, close_frame));
}

fn is_close_for(text: &str, sub: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(text).is_ok_and(|v| {
        v.as_array().is_some_and(|a| {
            a.first().and_then(|t| t.as_str()) == Some("CLOSE")
                && a.get(1).and_then(|i| i.as_str()) == Some(sub)
        })
    })
}
