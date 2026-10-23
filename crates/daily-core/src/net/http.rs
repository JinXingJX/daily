use crate::fs_utils::read_limited_utf8;
use crate::limits::MAX_CA_BUNDLE_BYTES;
use crate::net::ssrf::is_blocked_resolved_ip;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use reqwest::redirect::Policy;
use reqwest::{Certificate, Client};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

pub(crate) const CODEX_CA_CERTIFICATE_ENV: &str = "CODEX_CA_CERTIFICATE";
pub(crate) const SSL_CERT_FILE_ENV: &str = "SSL_CERT_FILE";

pub(crate) fn build_http_client() -> Result<Client, String> {
    build_http_client_with_redirects(true)
}

pub(crate) fn build_plugin_http_client() -> Result<Client, String> {
    build_http_client_with_redirects(false)
}

pub(crate) fn build_http_client_with_redirects(follow_redirects: bool) -> Result<Client, String> {
    // A model generating a long tool call sends nothing for minutes, and proxy nodes or
    // CDNs drop silent tunnels. HTTP/2 pings travel end to end inside TLS, so they keep
    // every hop busy; TCP keepalive only reaches the first one. A short pool idle time
    // avoids reusing connections the far end has likely closed.
    let mut builder = Client::builder()
        .http2_keep_alive_interval(Duration::from_secs(15))
        .http2_keep_alive_timeout(Duration::from_secs(20))
        .tcp_keepalive(Duration::from_secs(30))
        .pool_idle_timeout(Duration::from_secs(30));

    if follow_redirects {
        let cookie_store = Arc::new(reqwest::cookie::Jar::default());
        builder = builder.cookie_provider(cookie_store);
    } else {
        builder = builder
            .redirect(Policy::none())
            .no_proxy()
            .dns_resolver(Arc::new(PublicDnsResolver));
    }

    if let Some((source_env, path)) = configured_ca_bundle_path() {
        let label = format!("CA bundle from {source_env}");
        let pem = read_limited_utf8(&path, MAX_CA_BUNDLE_BYTES, &label)?.into_bytes();
        let certificates = Certificate::from_pem_bundle(&pem).map_err(|error| {
            format!(
                "Failed to parse CA bundle from {source_env}={}: {error}",
                path.display()
            )
        })?;
        if certificates.is_empty() {
            return Err(format!(
                "CA bundle from {source_env}={} did not contain any certificates.",
                path.display()
            ));
        }
        for certificate in certificates {
            builder = builder.add_root_certificate(certificate);
        }
    }

    builder
        .build()
        .map_err(|error| format!("Failed to build HTTP client: {error}"))
}

#[derive(Debug)]
pub(crate) struct PublicDnsResolver;

impl Resolve for PublicDnsResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            let addresses = tokio::net::lookup_host((host.as_str(), 0))
                .await
                .map_err(|error| Box::new(error) as Box<dyn std::error::Error + Send + Sync>)?
                .collect::<Vec<_>>();
            if addresses.is_empty() {
                return Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("DNS did not return an address for {host}."),
                ))
                    as Box<dyn std::error::Error + Send + Sync>);
            }
            if addresses
                .iter()
                .any(|address| is_blocked_resolved_ip(address.ip()))
            {
                return Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    format!("DNS for {host} resolved to a private or non-public address."),
                ))
                    as Box<dyn std::error::Error + Send + Sync>);
            }
            Ok(Box::new(addresses.into_iter()) as Addrs)
        })
    }
}

pub(crate) fn configured_ca_bundle_path() -> Option<(&'static str, PathBuf)> {
    for source_env in [CODEX_CA_CERTIFICATE_ENV, SSL_CERT_FILE_ENV] {
        if let Ok(value) = std::env::var(source_env) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some((source_env, PathBuf::from(trimmed)));
            }
        }
    }
    None
}
