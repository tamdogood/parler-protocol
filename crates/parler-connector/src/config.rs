//! Local agent state: the hub URL, the agent's display name/role, and its nkey identity.
//!
//! Persisted to `$PARLER_HOME/config.json` (default `~/.parler/config.json`) with `0600` perms —
//! it holds the nkey **seed** (the private half of the identity), which never goes on the wire.

use anyhow::{Context, Result};
use parler_auth::{new_identity, Identity};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Serialize, Deserialize)]
struct ConfigFile {
    hub_url: String,
    id: String,
    seed: String,
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    role: Option<String>,
}

// `seed` is private key material — keep it out of any `{:?}` / log line (mirrors `Identity`'s Debug).
impl std::fmt::Debug for ConfigFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConfigFile")
            .field("hub_url", &self.hub_url)
            .field("id", &self.id)
            .field("seed", &"<redacted>")
            .field("name", &self.name)
            .field("role", &self.role)
            .finish()
    }
}

/// The agent's local configuration + identity.
#[derive(Debug, Clone)]
pub struct Config {
    pub hub_url: String,
    pub identity: Identity,
    pub name: String,
    pub role: Option<String>,
}

/// The Parler Protocol home directory: `$PARLER_HOME`, else `~/.parler`.
pub fn home_dir() -> PathBuf {
    if let Ok(p) = std::env::var("PARLER_HOME") {
        return PathBuf::from(p);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".parler")
}

fn config_path() -> PathBuf {
    home_dir().join("config.json")
}

impl Config {
    /// Create a fresh identity + config (not yet saved).
    pub fn create(hub_url: impl Into<String>, name: impl Into<String>, role: Option<String>) -> Result<Config> {
        Ok(Config {
            hub_url: hub_url.into(),
            identity: new_identity()?,
            name: name.into(),
            role,
        })
    }

    /// Load the saved config, or a helpful error pointing at `parler init`.
    pub fn load() -> Result<Config> {
        let path = config_path();
        let data = std::fs::read_to_string(&path).with_context(|| {
            format!("no Parler Protocol identity at {} — run `parler init` first", path.display())
        })?;
        let f: ConfigFile = serde_json::from_str(&data).context("parsing config.json")?;
        Ok(Config {
            hub_url: f.hub_url,
            identity: Identity { id: f.id, seed: f.seed },
            name: f.name,
            role: f.role,
        })
    }

    pub fn exists() -> bool {
        config_path().exists()
    }

    /// Persist to `$PARLER_HOME/config.json`, owner-only (`0600`) — it holds the private seed, so the
    /// write is atomic (temp file + rename) and never leaves the seed at the default umask.
    pub fn save(&self) -> Result<()> {
        let f = ConfigFile {
            hub_url: self.hub_url.clone(),
            id: self.identity.id.clone(),
            seed: self.identity.seed.clone(),
            name: self.name.clone(),
            role: self.role.clone(),
        };
        let path = config_path();
        let body = serde_json::to_string_pretty(&f)?;
        parler_auth::write_private_file(&path, body.as_bytes())
            .with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }
}
