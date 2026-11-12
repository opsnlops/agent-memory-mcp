//! Shared long-term memory for coding agents, served over MCP.

pub mod server;
pub mod store;

use std::sync::Arc;

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
    if opts.allow_any_host {
        config = config.disable_allowed_hosts();
    } else {
        let hosts = LOOPBACK_HOSTS
            .into_iter()
            .map(String::from)
            .chain(opts.allowed_hosts.iter().map(|h| h.trim().to_string()))
            .filter(|h| !h.is_empty());
        config = config.with_allowed_hosts(hosts);
    }
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
