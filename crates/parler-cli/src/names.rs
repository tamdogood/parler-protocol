//! Fun, human-friendly default agent names.
//!
//! A fresh agent used to register as `$USER` or `<host>-<user>` (e.g. `codex-tam`,
//! `claude-code-tam`). Functional, but drab — and the shared directory reads like a spreadsheet.
//! [`fun_name`] gives each agent a playful `adjective-animal-<tag>` handle (e.g. `mellow-otter-a3f2`)
//! instead.
//!
//! It is **deterministic** in its seed, on purpose: re-running `parler connect` for the same host +
//! user must produce the same name (idempotent wiring — a re-run mustn't silently rename the agent),
//! and a bootstrapped identity keyed on its stable nkey id keeps its name across restarts. The short
//! hex `-<tag>` at the end is derived from the same seed, so two different seeds (a different user,
//! host, or minted id) almost always land on distinct handles even when they share the animal.

/// Playful adjectives — kept lowercase, single-token, and directory-safe (`[a-z]`).
const ADJECTIVES: &[&str] = &[
    "mellow", "swift", "brave", "clever", "sunny", "witty", "nimble", "cosmic", "plucky", "zesty",
    "breezy", "jolly", "quirky", "spry", "bold", "curious", "dapper", "eager", "fuzzy", "gutsy",
    "keen", "lucky", "peppy", "rowdy", "snappy", "chipper", "dashing", "frisky", "merry", "wily",
    "gallant", "spirited",
];

/// Playful animals — same constraints, so `adjective-animal` is always a clean handle.
const ANIMALS: &[&str] = &[
    "otter", "panda", "falcon", "lynx", "koala", "heron", "badger", "gecko", "walrus", "marmot",
    "puffin", "meerkat", "narwhal", "octopus", "raccoon", "wombat", "ferret", "hedgehog", "beaver",
    "mongoose", "capybara", "platypus", "pangolin", "axolotl", "quokka", "tapir", "ibex", "fennec",
    "manatee", "osprey", "salmon", "sparrow",
];

/// A stable, fun `adjective-animal-<tag>` name derived from `seed`.
///
/// Same seed → same name (idempotent). The `-<tag>` is a short hex fingerprint of the seed, so
/// distinct seeds stay distinct even when the adjective/animal happen to collide.
pub(crate) fn fun_name(seed: &str) -> String {
    let h = fnv1a(seed.as_bytes());
    let adjective = ADJECTIVES[(h % ADJECTIVES.len() as u64) as usize];
    let animal = ANIMALS[((h / ADJECTIVES.len() as u64) % ANIMALS.len() as u64) as usize];
    // 16-bit hex fingerprint pulled from a different slice of the hash than the word indices, so the
    // tag varies independently of the chosen words.
    let tag = ((h >> 40) & 0xffff) as u16;
    format!("{adjective}-{animal}-{tag:04x}")
}

/// FNV-1a (64-bit) — a tiny, dependency-free, deterministic string hash. Not cryptographic; we only
/// need a stable spread over the wordlists and the hex tag.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_deterministic_in_the_seed() {
        assert_eq!(fun_name("codex-tam"), fun_name("codex-tam"));
    }

    #[test]
    fn distinct_seeds_get_distinct_names() {
        assert_ne!(fun_name("codex-tam"), fun_name("claude-code-tam"));
        assert_ne!(fun_name("UABCD1234"), fun_name("UWXYZ9876"));
    }

    #[test]
    fn is_a_clean_lowercase_handle() {
        let name = fun_name("some-agent-id");
        assert!(
            name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
            "handle must be [a-z0-9-]: {name}"
        );
        // adjective-animal-tag → three dash-separated parts, the last a 4-char hex tag.
        let parts: Vec<&str> = name.split('-').collect();
        assert_eq!(parts.len(), 3, "shape is adjective-animal-tag: {name}");
        assert_eq!(parts[2].len(), 4, "tag is 4 hex chars: {name}");
        assert!(ADJECTIVES.contains(&parts[0]));
        assert!(ANIMALS.contains(&parts[1]));
    }
}
