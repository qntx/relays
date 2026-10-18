//! NIP-11 relay information document fetches.

use anyhow::{Context as _, Result, bail};
use futures::StreamExt;
use relays_core::url::RelayUrl;

use super::{NIP11_MAX_BODY, Prober};

/// Fetch a relay's NIP-11 document. Only 2xx responses whose body is a JSON
/// document of at most 64 KiB are accepted.
///
/// `reqwest` does not call its custom DNS resolver for IP-literal hosts, so
/// the address policy is enforced here before any request is made.
///
/// # Errors
///
/// Returns an error on a policy rejection, transport failure, non-2xx
/// status, oversized bodies, or non-JSON documents.
pub(crate) async fn fetch(prober: &Prober, url: &RelayUrl) -> Result<String> {
    if let Some(ip) = url.ip()
        && !prober.policy.allows(ip)
    {
        bail!("address policy rejects {ip} for {url}");
    }
    let response = prober
        .http
        .get(url.nip11_url())
        .header("Accept", "application/nostr+json")
        .send()
        .await
        .with_context(|| format!("NIP-11 fetch of {url} failed"))?;
    let status = response.status();
    if !status.is_success() {
        bail!("NIP-11 fetch of {url} returned {status}");
    }
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("NIP-11 response stream failed")?;
        body.extend_from_slice(&chunk);
        if body.len() > NIP11_MAX_BODY {
            bail!("NIP-11 document of {url} exceeds 64 KiB");
        }
    }
    let text = String::from_utf8(body).context("NIP-11 document is not UTF-8")?;
    serde_json::from_str::<serde_json::Value>(&text)
        .with_context(|| format!("NIP-11 document of {url} is not JSON"))?;
    Ok(text)
}
