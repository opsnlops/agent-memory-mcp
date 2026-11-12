use std::{io::IsTerminal, net::SocketAddr};

use agent_memory_mcp::{HttpOptions, app, host_allowlist_warning, store::Store};
use anyhow::{Context, Result};
use clap::Parser;
use tracing_subscriber::EnvFilter;

/// Shared memory MCP server for coding agents, backed by MongoDB.
#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    /// MongoDB connection string.
    #[arg(long, env = "MONGODB_URI", default_value = "mongodb://127.0.0.1:27017")]
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
    // No color codes when logging to journald or a file.
    tracing_subscriber::fmt()
        .with_ansi(std::io::stdout().is_terminal())
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();

    // Report Host allow-list problems before connecting, so they show up even
    // when MongoDB is slow or down.
    let http = HttpOptions {
        allowed_hosts: args.allowed_hosts,
        allow_any_host: args.allow_any_host,
    };
    match http.effective_allowed_hosts() {
        Some(hosts) => tracing::info!("accepting Host headers: {}", hosts.join(", ")),
        None => tracing::warn!("accepting any Host header (DNS-rebinding protection disabled)"),
    }
    if let Some(warning) = host_allowlist_warning(args.bind, &http) {
        tracing::warn!("{warning}");
    }

    let store = Store::connect(&args.mongodb_uri, &args.database).await?;
    tracing::info!(uri = %args.mongodb_uri, db = %args.database, "connected to MongoDB");

    let (router, shutdown) = app(store, &http);

    let listener = tokio::net::TcpListener::bind(args.bind)
        .await
        .with_context(|| format!("binding {}", args.bind))?;
    tracing::info!("listening on http://{}/mcp", listener.local_addr()?);

    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            tracing::info!("shutting down");
            shutdown.cancel();
        })
        .await?;
    Ok(())
}

/// Resolve on Ctrl-C or SIGTERM (what systemd sends on stop).
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = term => {},
    }
}
