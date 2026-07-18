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
#[command(name = "parler-hub", about = "Parler Protocol Hub — the lightweight bus for agent-to-agent messaging")]
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
    #[arg(long, env = "PARLER_HUB_NAME", default_value = "Parler Protocol Hub")]
    name: String,

    /// Run a public hub: public cards are world-readable. Private cards and the hub-scope view still
    /// require a directory token. Omit for a private hub.
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

    /// Largest structured JSON WebSocket frame, in bytes (default 2 MiB).
    #[arg(long, env = "PARLER_HUB_MAX_TEXT_FRAME_BYTES")]
    max_text_frame_bytes: Option<usize>,

    /// Aggregate bytes reserved by concurrent blob uploads (default 50 MiB).
    #[arg(long, env = "PARLER_HUB_MAX_INFLIGHT_BLOB_BYTES")]
    max_inflight_blob_bytes: Option<usize>,

    /// Ceiling on concurrent connections (default 1024).
    #[arg(long, env = "PARLER_HUB_MAX_CONNECTIONS")]
    max_connections: Option<usize>,

    /// Durable owned-room quota per identity (default 1000).
    #[arg(long, env = "PARLER_HUB_MAX_OWNED_ROOMS")]
    max_owned_rooms: Option<u64>,

    /// Active directory/watch token quota per identity (default 1000).
    #[arg(long, env = "PARLER_HUB_MAX_ACTIVE_TOKENS")]
    max_active_tokens: Option<u64>,

    /// Distinct keyed-memory quota per identity (default 10000; existing keys remain updatable).
    #[arg(long, env = "PARLER_HUB_MAX_KEYED_FACTS")]
    max_keyed_facts: Option<u64>,

    /// Per-client-IP HTTP request budget per minute across the REST/A2A endpoints and the `/ws`
    /// upgrade — the anti-abuse guard for the public front door (default 600). Pass `0` to disable.
    #[arg(long, env = "PARLER_HUB_MAX_HTTP_PER_MIN")]
    max_http_per_min: Option<u32>,

    /// Authenticated WebSocket operation budget per agent per minute (default 600).
    #[arg(long, env = "PARLER_HUB_MAX_OPS_PER_MIN")]
    max_ops_per_min: Option<u32>,

    /// Message-send budget per agent per minute (default 240).
    #[arg(long, env = "PARLER_HUB_MAX_SENDS_PER_MIN")]
    max_sends_per_min: Option<u32>,

    /// Blob-upload budget per agent per hour (default 120).
    #[arg(long, env = "PARLER_HUB_MAX_BLOBS_PER_HOUR")]
    max_blobs_per_hour: Option<u32>,

    /// Trust `Fly-Client-IP` / `X-Forwarded-For` for rate limiting. Enable only behind a proxy that
    /// overwrites these headers; direct deployments must use the socket peer address.
    #[arg(long, env = "PARLER_HUB_TRUST_PROXY_HEADERS")]
    trust_proxy_headers: bool,

    /// Per-**room** send budget per minute — the aggregate of every member — so one busy/abusive room
    /// can't monopolize the shared SQLite writer and stall other rooms (default 1200). Pass `0` to
    /// disable. Sits on top of the per-agent send limit.
    #[arg(long, env = "PARLER_HUB_MAX_ROOM_SENDS_PER_MIN")]
    max_room_sends_per_min: Option<u32>,

    /// Per-**room** blob-upload budget per hour — bounds how fast one room can consume the shared blob
    /// disk budget, so a single room can't fill storage for everyone (default 600). Pass `0` to disable.
    #[arg(long, env = "PARLER_HUB_MAX_ROOM_BLOBS_PER_HOUR")]
    max_room_blobs_per_hour: Option<u32>,

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
    /// Defaults to 30 days; pass `0` to keep all message history.
    #[arg(long, env = "PARLER_HUB_RETENTION_DAYS")]
    retention_days: Option<u64>,

    /// Retention floor: always keep at least this many newest messages per room (default 10000).
    #[arg(long, env = "PARLER_HUB_KEEP_MESSAGES_PER_ROOM")]
    keep_messages_per_room: Option<i64>,

    /// Retention: keep only this many newest unkeyed facts per (author, room). Defaults to 500; pass a
    /// negative value to keep all.
    #[arg(long, env = "PARLER_HUB_KEEP_FACTS")]
    keep_facts: Option<i64>,

    /// Retention: garbage-collect blob bytes neither fetched nor created within this many days.
    /// Defaults to 14 days; pass `0` to keep blobs until the disk budget fills.
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
    if let Some(max) = args.max_text_frame_bytes {
        state.max_text_frame_bytes = max;
    }
    if let Some(max) = args.max_inflight_blob_bytes {
        state.set_max_inflight_blob_bytes(max);
    }
    if let Some(max) = args.max_connections {
        state.max_connections = max;
    }
    if let Some(max) = args.max_owned_rooms {
        state.max_owned_rooms = max;
    }
    if let Some(max) = args.max_active_tokens {
        state.max_active_tokens = max;
    }
    if let Some(max) = args.max_keyed_facts {
        state.max_keyed_facts = max;
    }
    if let Some(max) = args.max_http_per_min {
        state.max_http_per_min = max;
    }
    if let Some(max) = args.max_ops_per_min {
        state.limits.max_ops_per_min = max;
    }
    if let Some(max) = args.max_sends_per_min {
        state.limits.max_sends_per_min = max;
    }
    if let Some(max) = args.max_blobs_per_hour {
        state.limits.max_blobs_per_hour = max;
    }
    state.trust_proxy_headers = args.trust_proxy_headers;
    if let Some(max) = args.max_room_sends_per_min {
        state.limits.max_room_sends_per_min = max;
    }
    if let Some(max) = args.max_room_blobs_per_hour {
        state.limits.max_room_blobs_per_hour = max;
    }
    state.join_secret = resolve_join_secret(
        args.join_secret,
        args.join_secret_file.as_deref().map(std::path::Path::new),
    )?;
    if mode == HubMode::Private && state.join_secret.is_none() && !loopback_bind(&args.addr) {
        anyhow::bail!(
            "refusing to expose a private hub on '{}' without a join secret; set \
             PARLER_HUB_JOIN_SECRET/--join-secret, use --join-secret-file, or bind to loopback",
            args.addr
        );
    }

    let defaults = Retention::default();
    let days_to_dur = |d: u64| Duration::from_secs(d * 24 * 3600);
    // Absent flag ⇒ the (now non-trivial) default; an explicit `0` (or negative `keep_facts`) ⇒
    // keep-everything, so an operator can still opt out of trimming entirely.
    state.retention = Retention {
        message_max_age: match args.retention_days {
            None => defaults.message_max_age,
            Some(0) => None,
            Some(d) => Some(days_to_dur(d)),
        },
        keep_messages_per_room: args
            .keep_messages_per_room
            .map(|k| k.max(0))
            .unwrap_or(defaults.keep_messages_per_room),
        keep_unkeyed_facts: match args.keep_facts {
            None => defaults.keep_unkeyed_facts,
            Some(k) if k < 0 => None,
            Some(k) => Some(k),
        },
        blob_max_idle: match args.blob_ttl_days {
            None => defaults.blob_max_idle,
            Some(0) => None,
            Some(d) => Some(days_to_dur(d)),
        },
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
        // Pass the hub + secret as `-e` flags so they persist into the stored MCP server config; a
        // shell-env prefix in front of `claude mcp add` would NOT survive into the launched
        // `parler mcp` (issue #100). First, install the binary; then wire the agent.
        println!("    cargo install --git https://github.com/tamdogood/parler-protocol parler-bin\n");
        match &state.join_secret {
            Some(secret) => println!(
                "    claude mcp add parler \\\n      -e PARLER_HUB={connect_url} \\\n      -e PARLER_JOIN_SECRET={secret} \\\n      -- parler mcp\n"
            ),
            None => println!(
                "    claude mcp add parler -e PARLER_HUB={connect_url} -- parler mcp\n"
            ),
        }
    }

    serve(listener, state).await
}

fn loopback_bind(addr: &str) -> bool {
    addr.parse::<std::net::SocketAddr>()
        .map(|addr| addr.ip().is_loopback())
        .unwrap_or_else(|_| {
            addr.rsplit_once(':')
                .map(|(host, _)| host.eq_ignore_ascii_case("localhost"))
                .unwrap_or(false)
        })
}

#[cfg(test)]
mod tests {
    use super::loopback_bind;

    #[test]
    fn only_explicit_loopback_binds_are_treated_as_local() {
        assert!(loopback_bind("127.0.0.1:7070"));
        assert!(loopback_bind("[::1]:7070"));
        assert!(loopback_bind("localhost:7070"));
        assert!(!loopback_bind("0.0.0.0:7070"));
        assert!(!loopback_bind("[::]:7070"));
        assert!(!loopback_bind("hub.example.com:7070"));
    }
}
