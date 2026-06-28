//! `parler-hub` — run the agent bus.
//!
//! ```text
//! parler-hub --addr 127.0.0.1:7070 --db ~/.parler/hub.sqlite
//! ```

use clap::Parser;
use parler_hub::{serve, HubMode, HubState, Store};
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

    /// Display name for this hub (the workspace name shown in the directory/site).
    #[arg(long, env = "PARLER_HUB_NAME", default_value = "Parler Hub")]
    name: String,

    /// Run a public hub: its directory is world-readable (no token needed for the hub-scope view).
    /// Omit for a private hub, where the full directory is gated behind a directory token.
    #[arg(long, env = "PARLER_HUB_PUBLIC")]
    public: bool,

    /// Directory for handed-off blob bytes (code bundles). Defaults to `<db>.blobs/` next to the
    /// SQLite file, or a temp dir for an in-memory hub.
    #[arg(long, env = "PARLER_HUB_BLOB_DIR")]
    blob_dir: Option<String>,

    /// Largest single blob (git bundle) the hub accepts, in bytes (default 25 MiB).
    #[arg(long, env = "PARLER_HUB_MAX_BLOB_BYTES")]
    max_blob_bytes: Option<u64>,

    /// Total disk budget for all stored blobs, in bytes (default 1 GiB).
    #[arg(long, env = "PARLER_HUB_MAX_BLOB_DIR_BYTES")]
    max_blob_dir_bytes: Option<u64>,

    /// Largest single message's serialized parts, in bytes (default 1 MiB).
    #[arg(long, env = "PARLER_HUB_MAX_MESSAGE_BYTES")]
    max_message_bytes: Option<usize>,

    /// Ceiling on concurrent connections (default 1024).
    #[arg(long, env = "PARLER_HUB_MAX_CONNECTIONS")]
    max_connections: Option<usize>,

    /// Require this shared secret on connect (recommended for a private hub that is reachable over a
    /// public URL — without it, anyone who can reach the hub can join). Agents present it via
    /// `PARLER_JOIN_SECRET`.
    #[arg(long, env = "PARLER_HUB_JOIN_SECRET")]
    join_secret: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()))
        .init();

    let args = Args::parse();
    let store = Store::open(args.db.as_deref().map(std::path::Path::new))?;
    let public_url = args.url.unwrap_or_else(|| format!("parler://{}", args.addr));
    let mode = if args.public { HubMode::Public } else { HubMode::Private };
    let mut state = HubState::new(store, public_url, args.name, mode);
    if let Some(dir) = args.blob_dir {
        state.blob_dir = std::path::PathBuf::from(dir);
    } else if let Some(db) = &args.db {
        state.blob_dir = std::path::PathBuf::from(format!("{db}.blobs"));
    }
    if let Some(max) = args.max_blob_bytes {
        state.max_blob_bytes = max;
    }
    if let Some(max) = args.max_blob_dir_bytes {
        state.max_blob_dir_bytes = max;
    }
    if let Some(max) = args.max_message_bytes {
        state.max_message_bytes = max;
    }
    if let Some(max) = args.max_connections {
        state.max_connections = max;
    }
    state.join_secret = args.join_secret.filter(|s| !s.is_empty());
    let state = Arc::new(state);

    let listener = tokio::net::TcpListener::bind(&args.addr).await?;
    let actual = listener.local_addr()?;
    tracing::info!("parler-hub listening on ws://{actual}/ws");
    println!(
        "parler-hub up · ws://{actual}/ws · {} hub '{}' · db: {}",
        state.mode.as_str(),
        state.name,
        args.db.as_deref().unwrap_or(":memory:")
    );

    serve(listener, state).await
}
