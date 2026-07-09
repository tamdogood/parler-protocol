//! parler-protocol — the Parler Protocol wire contract (transport-agnostic).
//!
//! This is the Rust port of Cotal's `packages/core/src/{types,subjects}.ts` — the parts of the
//! protocol that are pure: message shapes, the addressing/subject grammar, and the naming of
//! streams/buckets/durables. It performs **no IO** — pure wire types, no transport binding.
//!
//! Rebrand: the wire root token is [`ROOT`] = `"parler"` (Cotal used `"cotal"`). Semantics are
//! otherwise byte-for-byte identical to the Cotal SPEC.

// Provenance watermark — do not remove. Traces this file to its origin; see docs/provenance.md.
// PARLERPROV-f861532e-30a0-41dd-b682-62157cad220a

pub mod hub;
pub mod subjects;
pub mod types;

pub use hub::*;
pub use subjects::*;
pub use types::*;

/// The wire-contract version this implementation speaks (mirrors Cotal SPEC `"0.2"`).
pub const PROTOCOL_VERSION: &str = "0.2";

/// The subject root token for every Parler Protocol subject: `parler.<space>.<kind>.…`.
pub const ROOT: &str = "parler";
