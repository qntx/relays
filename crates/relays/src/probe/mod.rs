//! Relay probing: DNS resolution, WebSocket open/read checks and NIP-11
//! document fetches.

pub mod nip11;
pub mod resolve;
pub mod ws;

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result};
use hickory_resolver::TokioResolver;
use relays_core::observe::Check;
use relays_core::url::RelayUrl;
use rustls::ClientConfig;

use self::resolve::{AddressPolicy, HttpResolver};

/// DNS resolution timeout.
pub(crate) const DNS_TIMEOUT: Duration = Duration::from_secs(5);
/// Budget from start of DNS resolution to the HTTP 101 response.
pub(crate) const OPEN_TIMEOUT: Duration = Duration::from_secs(15);
/// Bound on a single TCP connect attempt inside the open budget.
pub(crate) const CONNECT_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(5);
/// Budget for the read check after the upgrade.
pub(crate) const READ_TIMEOUT: Duration = Duration::from_secs(10);
/// Budget for the closing handshake.
pub(crate) const CLOSE_TIMEOUT: Duration = Duration::from_secs(1);
/// NIP-11 fetch timeout.
pub(crate) const NIP11_TIMEOUT: Duration = Duration::from_secs(10);
/// NIP-11 body size limit.
pub(crate) const NIP11_MAX_BODY: usize = 64 * 1024;
/// Maximum WebSocket frame and message size.
pub(crate) const MAX_MESSAGE: usize = 256 * 1024;
/// `User-Agent` sent with upgrade and NIP-11 requests.
pub(crate) const USER_AGENT: &str = concat!(
    "relays/",
    env!("CARGO_PKG_VERSION"),
    " (+https://relays.meowl.social)"
);

/// The probe entry point, built once and shared across all checks.
///
/// Owns the fixed-upstream resolver, the address policy, a shared
/// `rustls::ClientConfig` (webpki roots, `ring` provider) and the
/// `reqwest::Client` reusing that same TLS configuration.
#[derive(Debug)]
pub struct Prober {
    pub(crate) resolver: TokioResolver,
    pub(crate) policy: AddressPolicy,
    pub(crate) tls: Arc<ClientConfig>,
    http: reqwest::Client,
}

impl Prober {
    /// Build the resolver, TLS configuration and HTTP client.
    ///
    /// # Errors
    ///
    /// Returns an error when the resolver or HTTP client cannot be
    /// constructed.
    pub fn new(policy: AddressPolicy) -> Result<Self> {
        let mut roots = rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let tls =
            ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()
                .context("failed to configure TLS")?
                .with_root_certificates(roots)
                .with_no_client_auth();
        let resolver = resolve::resolver().context("failed to build resolver")?;
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .dns_resolver(Arc::new(HttpResolver::new(resolver.clone(), policy)))
            .timeout(NIP11_TIMEOUT)
            .user_agent(USER_AGENT)
            .tls_backend_preconfigured(tls.clone())
            .build()
            .context("failed to build HTTP client")?;
        Ok(Self {
            resolver,
            policy,
            tls: Arc::new(tls),
            http,
        })
    }

    /// Run one check against `url`: DNS, address policy, TCP, optional TLS,
    /// WebSocket upgrade and the read check.
    pub async fn check(&self, url: &RelayUrl) -> Check {
        ws::check(self, url).await
    }

    /// Fetch a relay's NIP-11 document, returning the raw JSON text.
    ///
    /// # Errors
    ///
    /// Returns an error when the address policy rejects the host, on
    /// transport failure, non-2xx status, oversized bodies, or non-JSON
    /// documents.
    pub async fn nip11(&self, url: &RelayUrl) -> Result<String> {
        nip11::fetch(self, url).await
    }

    /// The shared HTTP client: fixed-upstream DNS, address-policy enforced,
    /// no redirects.
    #[must_use]
    pub const fn http(&self) -> &reqwest::Client {
        &self.http
    }
}
