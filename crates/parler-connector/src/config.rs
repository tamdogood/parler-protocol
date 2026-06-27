//! Local agent state: the hub URL, the agent's display name/role, and its nkey identity.
//!
//! Persisted to `$PARLER_HOME/config.json` (default `~/.parler/config.json`) with `0600` perms —
//! it holds the nkey **seed** (the private half of the identity), which never goes on the wire.

use anyhow::{Context, Result};
use parler_auth::{new_identity, Identity};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ConfigFile {
    hub_url: String,
    id: String,
    seed: String,
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    role: Option<String>,
}

/// The agent's local configuration + identity.
#[derive(Debug, Clone)]
pub struct Config {
    pub hub_url: String,
    pub identity: Identity,
    pub name: String,
    pub role: Option<String>,
}

/// The Parler home directory: `$PARLER_HOME`, else `~/.parler`.
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
            format!("no Parler identity at {} — run `parler init` first", path.display())
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

    /// Persist to `$PARLER_HOME/config.json` with `0600` perms.
    pub fn save(&self) -> Result<()> {
        let dir = home_dir();
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let f = ConfigFile {
            hub_url: self.hub_url.clone(),
            id: self.identity.id.clone(),
            seed: self.identity.seed.clone(),
            name: self.name.clone(),
            role: self.role.clone(),
        };
        let path = config_path();
        std::fs::write(&path, serde_json::to_string_pretty(&f)?)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }
}
