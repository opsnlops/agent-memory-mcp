mod server;
mod store;

use std::{net::SocketAddr, sync::Arc};

use anyhow::{Context, Result};
use axum::{Router, routing::get};
use clap::Parser;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use tracing_subscriber::EnvFilter;

use crate::{server::MemoryServer, store::Store};

/// Shared memory MCP server for coding agents, backed by MongoDB.
#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    /// MongoDB connection string.
    #[arg(
        long,
        env = "MONGODB_URI",
        default_value = "mongodb://fuzzball.local:27017"
    )]
    mongodb_uri: String,

    /// Database to store memories in.
    #[arg(long, env = "MONGODB_DATABASE", default_value = "agent_memory")]
    database: String,

    /// Address to listen on.
    #[arg(long, env = "BIND_ADDR", default_value = "0.0.0.0:8787")]
    bind: SocketAddr,

    /// Extra hostnames clients may use to reach this server (e.g. its Tailscale
    /// MagicDNS name). Loopback names are always allowed. Comma-separated.
    #[arg(long = "allowed-host", env = "ALLOWED_HOSTS", value_delimiter = ',')]
    allowed_hosts: Vec<String>,

    /// Accept any Host header. Disables DNS-rebinding protection.
    #[arg(long, env = "ALLOW_ANY_HOST")]
    allow_any_host: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();

    let store = Store::connect(&args.mongodb_uri, &args.database).await?;
    tracing::info!(uri = %args.mongodb_uri, db = %args.database, "connected to MongoDB");

    let mut config = StreamableHttpServerConfig::default();
    if args.allow_any_host {
        config = config.disable_allowed_hosts();
    } else {
        let hosts = ["localhost", "127.0.0.1", "::1"]
            .into_iter()
            .map(String::from)
            .chain(args.allowed_hosts.iter().map(|h| h.trim().to_string()))
            .filter(|h| !h.is_empty());
        config = config.with_allowed_hosts(hosts);
    }
    let shutdown = config.cancellation_token.clone();

    let mcp = StreamableHttpService::new(
        move || Ok(MemoryServer::new(store.clone())),
        Arc::new(LocalSessionManager::default()),
        config,
    );

    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .nest_service("/mcp", mcp);

    let listener = tokio::net::TcpListener::bind(args.bind)
        .await
        .with_context(|| format!("binding {}", args.bind))?;
    tracing::info!("listening on http://{}/mcp", args.bind);

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = tokio::signal::ctrl_c().await;
            shutdown.cancel();
        })
        .await?;
    Ok(())
}
