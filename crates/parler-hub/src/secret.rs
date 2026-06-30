//! Resolving the hub's optional join secret at boot.
//!
//! A private hub reachable beyond localhost should require a shared join secret (see the security
//! model in `AGENTS.md`): without it, anyone who can reach the hub can join. Operators can pass the
//! secret explicitly (`--join-secret` / `PARLER_HUB_JOIN_SECRET`), but for a turnkey private hub —
//! `docker compose up` and nothing else — we support a *secret file*: point `--join-secret-file` at
//! a path on the hub's volume and the hub generates a strong secret on first boot, persists it
//! `0600`, and reuses it on every later boot. That keeps the secret **stable across restarts** (so
//! agents that already have it keep working) and needs **no shell** (the runtime image is
//! distroless), so the whole private-hub setup stays one command.

use anyhow::{Context, Result};
use rand::Rng;
use std::path::Path;

/// Alphabet for a generated secret: omits visually ambiguous symbols (`0/O`, `1/I/l`) so it survives
/// a human copy-paste from a terminal banner into an MCP config.
const SECRET_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghjkmnpqrstuvwxyz23456789";

/// A high-entropy join secret: 32 chars over a 54-symbol alphabet (≈ 184 bits).
pub fn random_secret() -> String {
    let mut rng = rand::thread_rng();
    (0..32)
        .map(|_| SECRET_ALPHABET[rng.gen_range(0..SECRET_ALPHABET.len())] as char)
        .collect()
}

/// Decide the hub's join secret from the two operator knobs, in precedence order:
///   1. an explicit value (`--join-secret` / `PARLER_HUB_JOIN_SECRET`) — wins outright;
///   2. a secret *file* (`--join-secret-file`) — read it; if missing or empty, generate a strong
///      secret, persist it `0600`, and reuse it on every later boot;
///   3. neither — no secret (the binary's historical default; fine for a localhost-only hub).
///
/// Returns the resolved secret, if any. Blank values are treated as unset throughout.
pub fn resolve_join_secret(explicit: Option<String>, file: Option<&Path>) -> Result<Option<String>> {
    if let Some(s) = explicit.filter(|s| !s.is_empty()) {
        return Ok(Some(s));
    }
    let Some(path) = file.filter(|p| !p.as_os_str().is_empty()) else {
        return Ok(None);
    };

    // Reuse a previously generated secret so it stays stable across restarts.
    if let Ok(existing) = std::fs::read_to_string(path) {
        let existing = existing.trim().to_string();
        if !existing.is_empty() {
            return Ok(Some(existing));
        }
    }

    // First boot (or an empty file): mint one and persist it for next time.
    let secret = random_secret();
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(path, format!("{secret}\n"))
        .with_context(|| format!("writing join secret to {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(Some(secret))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_secret_has_expected_shape_and_varies() {
        let a = random_secret();
        assert_eq!(a.len(), 32);
        assert!(a.bytes().all(|b| SECRET_ALPHABET.contains(&b)));
        assert_ne!(a, random_secret(), "two draws should (overwhelmingly) differ");
    }

    #[test]
    fn explicit_value_wins_over_everything() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("join-secret");
        let got = resolve_join_secret(Some("hunter2".into()), Some(&path)).unwrap();
        assert_eq!(got.as_deref(), Some("hunter2"));
        assert!(!path.exists(), "an explicit secret must not touch the file");
    }

    #[test]
    fn blank_explicit_is_ignored() {
        let got = resolve_join_secret(Some("".into()), None).unwrap();
        assert_eq!(got, None);
    }

    #[test]
    fn no_explicit_no_file_means_no_secret() {
        assert_eq!(resolve_join_secret(None, None).unwrap(), None);
    }

    #[test]
    fn file_is_generated_then_reused_stably() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("join-secret");

        // First boot: generated + persisted (parent dirs created on the way).
        let first = resolve_join_secret(None, Some(&path)).unwrap().unwrap();
        assert_eq!(first.len(), 32);
        assert!(path.exists());
        assert_eq!(std::fs::read_to_string(&path).unwrap().trim(), first);

        // Later boots: the same secret, untouched.
        let second = resolve_join_secret(None, Some(&path)).unwrap().unwrap();
        assert_eq!(first, second, "the secret must stay stable across restarts");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "the secret file must be owner-only");
        }
    }

    #[test]
    fn empty_file_is_regenerated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("join-secret");
        std::fs::write(&path, "   \n").unwrap();
        let got = resolve_join_secret(None, Some(&path)).unwrap().unwrap();
        assert_eq!(got.len(), 32);
        assert_eq!(std::fs::read_to_string(&path).unwrap().trim(), got);
    }
}
