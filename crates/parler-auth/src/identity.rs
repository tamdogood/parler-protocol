//! A locally-generated agent identity (an nkey user keypair). Port of Cotal `identity.ts`.
//!
//! The public key is the **stable id** used identically everywhere — `card.id`, the subject-encoded
//! sender token, the JWT subject, and the DM durable name. The seed is the private half; it never
//! goes on the wire and is folded into a creds file the endpoint loads to authenticate as this id.

use crate::error::AuthError;
use data_encoding::{BASE64, BASE64URL_NOPAD, HEXLOWER};
use nkeys::KeyPair;
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Clone, PartialEq, Eq)]
pub struct Identity {
    /// User nkey public key (`U…`). The stable agent id.
    pub id: String,
    /// User nkey seed (`SU…`). Private — kept off the wire.
    pub seed: String,
}

// The seed is private key material: keep it out of any `{:?}` / log line. `Debug` shows the id (a
// public key, safe to print) and redacts the seed. Same reason `Identity` is never `Serialize`.
impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Identity")
            .field("id", &self.id)
            .field("seed", &"<redacted>")
            .finish()
    }
}

/// Generate a fresh user nkey identity locally.
pub fn new_identity() -> Result<Identity, AuthError> {
    let kp = KeyPair::new_user();
    let seed = kp.seed().map_err(|e| AuthError::Nkeys(e.to_string()))?;
    Ok(Identity {
        id: kp.public_key(),
        seed,
    })
}

/// Sign `msg` with an nkey `seed` (`SU…`), returning the base64 (standard) Ed25519 signature.
///
/// Used to self-sign an agent's discovery card: the signature is verifiable against the agent's id
/// (its public key), so a hub that stores the card cannot forge or tamper with it.
pub fn sign(seed: &str, msg: &[u8]) -> Result<String, AuthError> {
    let kp = KeyPair::from_seed(seed).map_err(|e| AuthError::Nkeys(e.to_string()))?;
    let sig = kp.sign(msg).map_err(|e| AuthError::Nkeys(e.to_string()))?;
    Ok(BASE64.encode(&sig))
}

/// Verify a base64 Ed25519 signature over `msg` against an nkey public key `id` (`U…`).
/// Returns `false` for a bad key, malformed signature, or a verification mismatch (never errors).
pub fn verify(id: &str, msg: &[u8], sig_b64: &str) -> bool {
    let Ok(kp) = KeyPair::from_public_key(id) else {
        return false;
    };
    let Ok(sig) = BASE64.decode(sig_b64.as_bytes()) else {
        return false;
    };
    kp.verify(msg, &sig).is_ok()
}

/// The **content address** of a blob: lowercase-hex SHA-256 of its bytes.
///
/// Used to name and verify artifacts handed off through a hub (e.g. git bundles): the id *is* the
/// hash, so a stored blob dedups by content and any consumer can re-verify the bytes match the id.
/// The hashing is defined here so the uploader (connector) and the verifier (hub) agree byte-for-byte.
pub fn content_id(bytes: &[u8]) -> String {
    HEXLOWER.encode(&Sha256::digest(bytes))
}

/// Atomically write `contents` to `path` as an **owner-only (`0600`)** file, creating parent dirs.
///
/// For on-disk secrets — an agent's nkey seed (`config.json`) and a hub's join secret. We write a
/// freshly-created temp file (opened `0600` from the very first byte) in the same directory and then
/// `rename` it over the destination. Because `rename` is atomic and the new inode is `0600`, the
/// secret is **never** momentarily group/world-readable — unlike `write`-then-`set_permissions`,
/// which leaves the file at the default umask (`~0644`) in the window between the two syscalls.
pub fn write_private_file(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty());
    if let Some(dir) = parent {
        std::fs::create_dir_all(dir)?;
    }
    let dir = parent.map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("secret");
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = dir.join(format!(".{file_name}.tmp.{}.{stamp}", std::process::id()));

    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }

    let write = (|| -> std::io::Result<()> {
        let mut f = opts.open(&tmp)?;
        f.write_all(contents)?;
        f.sync_all()
    })();
    if let Err(e) = write.and_then(|()| std::fs::rename(&tmp, path)) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

/// The stable id carried by a creds file: the agent's nkey public key, derived from the seed block
/// and cross-checked against the JWT subject (a mismatch ⇒ a corrupt/spliced creds file).
pub fn id_from_creds(creds: &str) -> Result<String, AuthError> {
    let seed = extract_block(creds, "USER NKEY SEED")
        .ok_or_else(|| AuthError::Creds("no user nkey seed block found".into()))?;
    let kp = KeyPair::from_seed(seed.trim()).map_err(|e| AuthError::Creds(format!("bad seed: {e}")))?;
    let id = kp.public_key();
    if let Some(jwt) = extract_block(creds, "NATS USER JWT") {
        if let Some(sub) = jwt_subject(jwt.trim()) {
            if sub != id {
                return Err(AuthError::Creds(format!(
                    "seed identity {id} != JWT subject {sub}"
                )));
            }
        }
    }
    Ok(id)
}

/// Extract the content between `-----BEGIN <label>-----` and `------END <label>------` (tolerant).
fn extract_block(creds: &str, label: &str) -> Option<String> {
    let begin = format!("BEGIN {label}");
    let end = format!("END {label}");
    let b = creds.find(&begin)?;
    let after_begin = creds[b..].find('\n')? + b + 1;
    let e_marker = creds[after_begin..].find(&end)? + after_begin;
    // Back up to the start of the line carrying the END marker so its leading dashes are excluded.
    let line_start = creds[..e_marker].rfind('\n').map(|n| n + 1).unwrap_or(after_begin);
    Some(creds[after_begin..line_start].trim().to_string())
}

/// Read the `sub` claim out of a NATS JWT (the middle base64url-nopad segment).
fn jwt_subject(jwt: &str) -> Option<String> {
    let payload = jwt.split('.').nth(1)?;
    let bytes = BASE64URL_NOPAD.decode(payload.as_bytes()).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    v.get("sub")?.as_str().map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_identity_is_a_user_key() {
        let id = new_identity().unwrap();
        assert!(id.id.starts_with('U'));
        assert!(id.seed.starts_with("SU"));
        // The seed re-derives the same public key.
        let kp = KeyPair::from_seed(&id.seed).unwrap();
        assert_eq!(kp.public_key(), id.id);
    }

    #[test]
    fn sign_verify_round_trips_and_rejects_tampering() {
        let id = new_identity().unwrap();
        let sig = sign(&id.seed, b"card-bytes").unwrap();
        assert!(verify(&id.id, b"card-bytes", &sig));
        // A different message, a different signer, or a garbled signature all fail closed.
        assert!(!verify(&id.id, b"tampered", &sig));
        assert!(!verify(&new_identity().unwrap().id, b"card-bytes", &sig));
        assert!(!verify(&id.id, b"card-bytes", "not-base64!!"));
    }

    fn uniq_dir(tag: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("parler-auth-{tag}-{}-{stamp}", std::process::id()))
    }

    #[test]
    fn write_private_file_is_0600_and_roundtrips() {
        let dir = uniq_dir("wpf");
        // Parent dirs are created on the way (the target sits two levels deep).
        let path = dir.join("nested").join("config.json");
        write_private_file(&path, b"top-secret-seed").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"top-secret-seed");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "a private file must be owner-only");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    // The whole point of the atomic rename: overwriting an old, world-readable config must not
    // inherit its loose permissions (the bug that `write`-then-chmod left a window for).
    #[cfg(unix)]
    #[test]
    fn write_private_file_replaces_a_world_readable_file_with_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = uniq_dir("wpf-overwrite");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        std::fs::write(&path, b"old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        write_private_file(&path, b"new-secret").unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), b"new-secret");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "atomic rename must replace, never widen");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn debug_redacts_the_seed() {
        let id = new_identity().unwrap();
        let shown = format!("{id:?}");
        assert!(shown.contains(&id.id), "the id (a public key) is safe to show");
        assert!(!shown.contains(&id.seed), "the private seed must never appear in Debug");
        assert!(shown.contains("<redacted>"));
    }
}
