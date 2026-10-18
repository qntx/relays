//! DNS resolution via fixed public upstreams and the outbound address
//! policy.

use std::error::Error;
use std::fmt;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use hickory_resolver::TokioResolver;
use hickory_resolver::config::{NameServerConfig, ResolverConfig};
use hickory_resolver::net::NetError;
use hickory_resolver::net::runtime::TokioRuntimeProvider;

use super::DNS_TIMEOUT;

/// Fixed public upstreams, contacted over both UDP and TCP; the host
/// `resolv.conf` is never read.
const UPSTREAMS: [IpAddr; 2] = [
    IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)),
    IpAddr::V4(Ipv4Addr::new(9, 9, 9, 9)),
];

/// A resolver built on the fixed upstreams.
///
/// # Errors
///
/// Returns the hickory error when the resolver cannot be built.
pub(crate) fn resolver() -> Result<TokioResolver, NetError> {
    let name_servers = UPSTREAMS
        .iter()
        .map(|ip| NameServerConfig::udp_and_tcp(*ip))
        .collect();
    let config = ResolverConfig::from_name_servers(name_servers);
    TokioResolver::builder_with_config(config, TokioRuntimeProvider::default()).build()
}

/// Which resolved addresses are acceptable to connect to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressPolicy {
    /// Only globally routable unicast addresses.
    PublicOnly,
    /// Additionally allow loopback addresses; for tests only.
    AllowLoopback,
}

impl AddressPolicy {
    /// Whether a single address may be connected to.
    #[must_use]
    pub fn allows(self, ip: IpAddr) -> bool {
        match ip {
            IpAddr::V4(a) => public_v4(a, matches!(self, Self::AllowLoopback)),
            IpAddr::V6(a) => public_v6(a, matches!(self, Self::AllowLoopback)),
        }
    }
}

fn public_v4(addr: Ipv4Addr, allow_loopback: bool) -> bool {
    let [a, b, c, _] = addr.octets();
    if allow_loopback && a == 127 {
        return true;
    }
    !(a == 0
        || a == 10
        || a == 127
        || (a == 100 && (64..=127).contains(&b))
        || (a == 169 && b == 254)
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && b == 0 && c == 0)
        || (a == 192 && b == 0 && c == 2)
        || (a == 192 && b == 168)
        || (a == 198 && (b & 0xFE) == 18)
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113)
        || a >= 224)
}

fn public_v6(addr: Ipv6Addr, allow_loopback: bool) -> bool {
    if let Some(mapped) = addr.to_ipv4_mapped() {
        return public_v4(mapped, allow_loopback);
    }
    let segments = addr.segments();
    if allow_loopback && addr == Ipv6Addr::LOCALHOST {
        return true;
    }
    !(addr.is_unspecified()
        || addr == Ipv6Addr::LOCALHOST
        || (segments[0] & 0xFE00) == 0xFC00
        || (segments[0] & 0xFFC0) == 0xFE80
        || (segments[0] & 0xFF00) == 0xFF00
        || (segments[0] == 0x2001 && segments[1] == 0x0DB8))
}

/// DNS resolution failure classification.
#[derive(Debug)]
pub(crate) enum DnsError {
    /// NXDOMAIN or no records.
    Nxdomain,
    /// SERVFAIL, timeout or transport failure.
    Transient(String),
}

impl fmt::Display for DnsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Nxdomain => f.write_str("NXDOMAIN or no A/AAAA records"),
            Self::Transient(message) => f.write_str(message),
        }
    }
}

/// Resolve a host to IP addresses; `literal` short-circuits DNS when the
/// host is an IP literal (see [`RelayUrl::ip`]).
///
/// # Errors
///
/// Returns [`DnsError`] on resolution failure; an empty answer counts as
/// `Nxdomain`.
pub(crate) async fn lookup(
    resolver: &TokioResolver,
    host: &str,
    literal: Option<IpAddr>,
) -> Result<Vec<IpAddr>, DnsError> {
    if let Some(ip) = literal {
        return Ok(vec![ip]);
    }
    match tokio::time::timeout(DNS_TIMEOUT, resolver.lookup_ip(host)).await {
        Ok(Ok(lookup)) => {
            let ips: Vec<IpAddr> = lookup.iter().collect();
            if ips.is_empty() {
                Err(DnsError::Nxdomain)
            } else {
                Ok(ips)
            }
        }
        Ok(Err(e)) if e.is_no_records_found() => Err(DnsError::Nxdomain),
        Ok(Err(e)) => Err(DnsError::Transient(e.to_string())),
        Err(_) => Err(DnsError::Transient("DNS resolution timed out".to_owned())),
    }
}

/// reqwest DNS backend sharing the fixed upstreams and the address policy.
#[derive(Debug, Clone)]
pub(crate) struct HttpResolver {
    resolver: TokioResolver,
    policy: AddressPolicy,
}

impl HttpResolver {
    /// Wrap a resolver.
    #[must_use]
    pub(crate) const fn new(resolver: TokioResolver, policy: AddressPolicy) -> Self {
        Self { resolver, policy }
    }
}

impl reqwest::dns::Resolve for HttpResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let resolver = self.resolver.clone();
        let policy = self.policy;
        Box::pin(async move {
            let ips = lookup(&resolver, name.as_str(), name.as_str().parse().ok())
                .await
                .map_err(|e| to_box_error(io::Error::other(e.to_string())))?;
            if ips.iter().any(|ip| !policy.allows(*ip)) {
                return Err(to_box_error(io::Error::other("private address")));
            }
            let addrs: reqwest::dns::Addrs =
                Box::new(ips.into_iter().map(|ip| SocketAddr::new(ip, 0)));
            Ok(addrs)
        })
    }
}

fn to_box_error(error: impl Error + Send + Sync + 'static) -> Box<dyn Error + Send + Sync> {
    Box::new(error)
}
