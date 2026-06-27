//! `parler-hub` — run the agent bus.
//!
//! ```text
//! parler-hub --addr 127.0.0.1:7070 --db ~/.parler/hub.sqlite
//! ```

use clap::Parser;
use parler_hub::{serve, HubState, Store};
use std::sync::Arc;

#[derive(Parser)]
#[command(name = "parler-hub", about = "Parler Hub — the lightweight bus for agent-to-agent messaging")]
struct Args {
    /// Address to bind, host:port.
    #[arg(long, env = "PARLER_HUB_ADDR", default_value = "127.0.0.1:7070")]
    addr: String,

    /// SQLite file for durable storage. Omit for in-memory (lost on exit).
    #[arg(long, env = "PARLER_HUB_DB")]
    db: Option<String>,

    /// Public base URL advertised in invite links. Defaults to `parler://<addr>`.
    #[arg(long, env = "PARLER_HUB_URL")]
    url: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()))
        .init();

    let args = Args::parse();
    let store = Store::open(args.db.as_deref().map(std::path::Path::new))?;
    let public_url = args.url.unwrap_or_else(|| format!("parler://{}", args.addr));
    let state = Arc::new(HubState { store, public_url });

    let listener = tokio::net::TcpListener::bind(&args.addr).await?;
    let actual = listener.local_addr()?;
    tracing::info!("parler-hub listening on ws://{actual}/ws");
    println!(
        "parler-hub up · ws://{actual}/ws · db: {}",
        args.db.as_deref().unwrap_or(":memory:")
    );

    serve(listener, state).await
}
