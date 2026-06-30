//! `parler-hub` — run the agent bus.
//!
//! ```text
//! parler-hub --addr 127.0.0.1:7070 --db ~/.parler/hub.sqlite
//! ```

use clap::Parser;
use parler_hub::{display_hub_url, resolve_join_secret, serve, HubMode, HubState, Retention, Store};
use std::sync::Arc;
use std::time::Duration;

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

    /// Turnkey alternative to `--join-secret`: read the join secret from this file, generating and
    /// persisting a strong one on first boot if the file is absent (then reusing it across restarts).
    /// Point it at a path on the hub's data volume for a one-command private hub. Ignored if
    /// `--join-secret` is also given (an explicit value wins).
    #[arg(long, env = "PARLER_HUB_JOIN_SECRET_FILE")]
    join_secret_file: Option<String>,

    /// Retention: delete messages older than this many days (always keeping the per-room floor below).
    /// Omit / `0` keeps all message history. A long-lived public hub should set this so the log can't
    /// grow without bound.
    #[arg(long, env = "PARLER_HUB_RETENTION_DAYS")]
    retention_days: Option<u64>,

    /// Retention floor: always keep at least this many newest messages per room (default 10000).
    #[arg(long, env = "PARLER_HUB_KEEP_MESSAGES_PER_ROOM")]
    keep_messages_per_room: Option<i64>,

    /// Retention: keep only this many newest unkeyed facts per (author, room). Omit keeps all.
    #[arg(long, env = "PARLER_HUB_KEEP_FACTS")]
    keep_facts: Option<i64>,

    /// Retention: garbage-collect blob bytes neither fetched nor created within this many days. Omit
    /// keeps blobs until the disk budget fills.
    #[arg(long, env = "PARLER_HUB_BLOB_TTL_DAYS")]
    blob_ttl_days: Option<u64>,

    /// How often the background janitor runs, in seconds (default 3600).
    #[arg(long, env = "PARLER_HUB_JANITOR_INTERVAL_SECS")]
    janitor_interval_secs: Option<u64>,
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
    state.join_secret = resolve_join_secret(
        args.join_secret,
        args.join_secret_file.as_deref().map(std::path::Path::new),
    )?;

    let defaults = Retention::default();
    let days_to_dur = |d: u64| Duration::from_secs(d * 24 * 3600);
    state.retention = Retention {
        message_max_age: args.retention_days.filter(|d| *d > 0).map(days_to_dur),
        keep_messages_per_room: args
            .keep_messages_per_room
            .map(|k| k.max(0))
            .unwrap_or(defaults.keep_messages_per_room),
        keep_unkeyed_facts: args.keep_facts.filter(|k| *k >= 0),
        blob_max_idle: args.blob_ttl_days.filter(|d| *d > 0).map(days_to_dur),
        interval: args
            .janitor_interval_secs
            .filter(|s| *s > 0)
            .map(Duration::from_secs)
            .unwrap_or(defaults.interval),
    };

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

    // Operator-only connect snippet (stdout/logs are not world-readable, so it's safe to include the
    // secret here — this is the one place the auto-generated secret is surfaced). Mirrors the public
    // hub's one-line onboarding so a private hub is just as easy to point an agent at.
    if state.mode == HubMode::Private {
        let connect_url = display_hub_url(&state.public_url);
        println!("\n  Connect an agent (Claude Code shown — Codex/Cursor take the same env):\n");
        match &state.join_secret {
            Some(secret) => println!(
                "    PARLER_HUB={connect_url} PARLER_JOIN_SECRET={secret} \\\n      claude mcp add parler -- parler mcp\n"
            ),
            None => println!("    PARLER_HUB={connect_url} claude mcp add parler -- parler mcp\n"),
        }
    }

    serve(listener, state).await
}
