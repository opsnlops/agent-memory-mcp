//! Shared long-term memory for coding agents, served over MCP.

pub mod server;
pub mod store;

use std::{net::SocketAddr, sync::Arc};

use axum::{Router, routing::get};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
pub use tokio_util::sync::CancellationToken;

use crate::{server::MemoryServer, store::Store};

/// Hostnames that are always accepted in the `Host` header.
pub const LOOPBACK_HOSTS: [&str; 3] = ["localhost", "127.0.0.1", "::1"];

/// HTTP-level options for the MCP endpoint.
#[derive(Debug, Clone, Default)]
pub struct HttpOptions {
    /// Extra hostnames clients may use, in addition to [`LOOPBACK_HOSTS`].
    pub allowed_hosts: Vec<String>,
    /// Accept any `Host` header (disables DNS-rebinding protection).
    pub allow_any_host: bool,
}

/// Build the axum app (`/mcp` and `/healthz`). Cancel the returned token to
/// close open MCP sessions during shutdown.
pub fn app(store: Store, opts: &HttpOptions) -> (Router, CancellationToken) {
    let mut config = StreamableHttpServerConfig::default();
    config = match opts.effective_allowed_hosts() {
        None => config.disable_allowed_hosts(),
        Some(hosts) => config.with_allowed_hosts(hosts),
    };
    let shutdown = config.cancellation_token.clone();

    let mcp = StreamableHttpService::new(
        move || Ok(MemoryServer::new(store.clone())),
        Arc::new(LocalSessionManager::default()),
        config,
    );

    let router = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .nest_service("/mcp", mcp);
    (router, shutdown)
}

impl HttpOptions {
    /// Every hostname accepted in the `Host` header, or `None` if any host is accepted.
    pub fn effective_allowed_hosts(&self) -> Option<Vec<String>> {
        if self.allow_any_host {
            return None;
        }
        Some(
            LOOPBACK_HOSTS
                .into_iter()
                .map(String::from)
                .chain(self.allowed_hosts.iter().map(|h| h.trim().to_string()))
                .filter(|h| !h.is_empty())
                .collect(),
        )
    }
}

/// A startup warning when the server is reachable from the network but the
/// Host allow-list will reject remote clients. Those clients get HTTP 403,
/// which MCP clients such as Claude Code report as an authentication or
/// sign-in failure, so the cause is easy to misread.
pub fn host_allowlist_warning(bind: SocketAddr, opts: &HttpOptions) -> Option<String> {
    let allowed = opts.effective_allowed_hosts()?;
    let ip = bind.ip();
    if ip.is_loopback() {
        return None;
    }
    let consequence = "remote MCP clients will get HTTP 403, which clients such as \
                       Claude Code report as an authentication/sign-in error";
    if ip.is_unspecified() {
        if allowed.len() > LOOPBACK_HOSTS.len() {
            return None;
        }
        return Some(format!(
            "listening on all interfaces ({bind}) but only loopback Host headers are allowed; \
             {consequence}. Set ALLOWED_HOSTS (or --allowed-host) to every name/IP clients use, \
             e.g. ALLOWED_HOSTS=10.0.0.5,myhost,myhost.tailnet.ts.net"
        ));
    }
    let ip_str = ip.to_string();
    if allowed.iter().any(|h| h == &ip_str) {
        return None;
    }
    Some(format!(
        "listening on {bind} but {ip_str} is not in the Host allow-list; clients connecting \
         via that address will get HTTP 403 ({consequence}). Add it with ALLOWED_HOSTS={ip_str}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(hosts: &[&str]) -> HttpOptions {
        HttpOptions {
            allowed_hosts: hosts.iter().map(|h| h.to_string()).collect(),
            allow_any_host: false,
        }
    }

    fn addr(s: &str) -> SocketAddr {
        s.parse().unwrap()
    }

    #[test]
    fn warns_for_all_interfaces_with_loopback_only() {
        let w = host_allowlist_warning(addr("0.0.0.0:8787"), &opts(&[])).unwrap();
        assert!(w.contains("ALLOWED_HOSTS"), "{w}");
        assert!(w.contains("403"), "{w}");
        assert!(host_allowlist_warning(addr("[::]:8787"), &opts(&[" "])).is_some());
    }

    #[test]
    fn quiet_for_all_interfaces_with_extra_hosts() {
        assert!(host_allowlist_warning(addr("0.0.0.0:8787"), &opts(&["10.19.63.10"])).is_none());
    }

    #[test]
    fn quiet_for_loopback_bind() {
        assert!(host_allowlist_warning(addr("127.0.0.1:8787"), &opts(&[])).is_none());
        assert!(host_allowlist_warning(addr("[::1]:8787"), &opts(&[])).is_none());
    }

    #[test]
    fn quiet_when_any_host_allowed() {
        let o = HttpOptions {
            allowed_hosts: vec![],
            allow_any_host: true,
        };
        assert!(host_allowlist_warning(addr("0.0.0.0:8787"), &o).is_none());
        assert!(o.effective_allowed_hosts().is_none());
    }

    #[test]
    fn warns_when_specific_bind_ip_not_allowed() {
        let w = host_allowlist_warning(addr("10.19.63.10:8787"), &opts(&["fuzzball"])).unwrap();
        assert!(w.contains("ALLOWED_HOSTS=10.19.63.10"), "{w}");
        assert!(
            host_allowlist_warning(
                addr("10.19.63.10:8787"),
                &opts(&["fuzzball", "10.19.63.10"])
            )
            .is_none()
        );
    }
}
