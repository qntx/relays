//! Canonical relay URLs.

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::net::IpAddr;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;
use url::Url;

/// Error returned when a string is not a valid relay URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("invalid relay URL")]
pub struct UrlError;

/// A relay URL in canonical form.
///
/// `ws`/`wss` scheme, lowercase host, no default port, root path kept as
/// `/`, trailing slash removed elsewhere, duplicate slashes merged, query
/// parameters sorted, fragment removed.
#[derive(Debug, Clone)]
pub struct RelayUrl {
    inner: Url,
}

impl RelayUrl {
    /// Parse and canonicalise a relay URL. Inputs without a scheme are
    /// treated as `wss://`; `http` and `https` map to `ws` and `wss`.
    ///
    /// # Errors
    ///
    /// Returns [`UrlError`] when the input is not a parseable URL or uses a
    /// scheme other than `ws`, `wss`, `http` or `https`.
    pub fn parse(input: &str) -> Result<Self, UrlError> {
        let owned;
        let text = if input.contains("://") {
            input
        } else {
            owned = format!("wss://{input}");
            &owned
        };
        let url = Url::parse(text).map_err(|_| UrlError)?;
        let secure = match url.scheme() {
            "ws" | "http" => false,
            "wss" | "https" => true,
            _ => return Err(UrlError),
        };
        let host = url.host_str().ok_or(UrlError)?;
        let port = url.port();

        let mut path = String::with_capacity(url.path().len());
        let mut slash = false;
        for ch in url.path().chars() {
            if ch == '/' && slash {
                continue;
            }
            slash = ch == '/';
            path.push(ch);
        }
        if path.len() > 1 && path.ends_with('/') {
            path.pop();
        }
        if path.is_empty() {
            path.push('/');
        }

        let mut pairs: Vec<(String, String)> = url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        pairs.sort_by(|a, b| a.0.cmp(&b.0));

        let mut repr = String::with_capacity(text.len() + 2);
        repr.push_str(if secure { "wss" } else { "ws" });
        repr.push_str("://");
        if !url.username().is_empty() {
            repr.push_str(url.username());
            if let Some(password) = url.password() {
                repr.push(':');
                repr.push_str(password);
            }
            repr.push('@');
        }
        repr.push_str(host);
        if let Some(p) = port {
            repr.push(':');
            repr.push_str(&p.to_string());
        }
        repr.push_str(&path);
        if !pairs.is_empty() {
            repr.push('?');
            repr.push_str(
                &url::form_urlencoded::Serializer::new(String::new())
                    .extend_pairs(pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())))
                    .finish(),
            );
        }

        let inner = Url::parse(&repr).map_err(|_| UrlError)?;
        Ok(Self { inner })
    }

    /// The canonical string form.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.inner.as_str()
    }

    /// The canonical URL.
    #[must_use]
    pub const fn inner(&self) -> &Url {
        &self.inner
    }

    /// The lowercase, IDNA-encoded host. IP literals keep their original
    /// spelling; IPv6 literals include the brackets.
    #[must_use]
    pub fn host(&self) -> &str {
        self.inner.host_str().unwrap_or("")
    }

    /// The host as an IP literal, or `None` for DNS names. Unlike
    /// `host().parse::<IpAddr>()`, this also recognises IPv6 literals, which
    /// `host()` returns with brackets.
    #[must_use]
    pub fn ip(&self) -> Option<IpAddr> {
        match self.inner.host() {
            Some(url::Host::Ipv4(addr)) => Some(IpAddr::V4(addr)),
            Some(url::Host::Ipv6(addr)) => Some(IpAddr::V6(addr)),
            _ => None,
        }
    }

    /// The port, defaulting to 443 for `wss` and 80 for `ws`.
    #[must_use]
    pub fn port(&self) -> u16 {
        self.inner
            .port_or_known_default()
            .unwrap_or_else(|| if self.is_secure() { 443 } else { 80 })
    }

    /// Whether the scheme is `wss`.
    #[must_use]
    pub fn is_secure(&self) -> bool {
        self.inner.scheme() == "wss"
    }

    /// The NIP-11 document URL: `ws` becomes `http`, `wss` becomes `https`,
    /// path unchanged.
    #[must_use]
    pub fn nip11_url(&self) -> Url {
        let mut url = self.inner.clone();
        url.set_scheme(if self.is_secure() { "https" } else { "http" })
            .unwrap_or_default();
        url
    }
}

impl fmt::Display for RelayUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for RelayUrl {
    type Err = UrlError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl PartialEq for RelayUrl {
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

impl Eq for RelayUrl {}

impl PartialOrd for RelayUrl {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for RelayUrl {
    fn cmp(&self, other: &Self) -> Ordering {
        self.inner.as_str().cmp(other.inner.as_str())
    }
}

impl Hash for RelayUrl {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.inner.hash(state);
    }
}

impl Serialize for RelayUrl {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for RelayUrl {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::parse(&s).map_err(serde::de::Error::custom)
    }
}
