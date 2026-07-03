//! parler-auth — identity + decentralized NATS JWT issuance, profile ACLs, and `nats-server` config.
//!
//! Port of Cotal `packages/core/src/{identity,provision}.ts`. A space is one NATS *account*; every
//! agent is a *user* in it. This crate mints the operator→account→user JWT trust chain, builds the
//! six default-deny profile ACLs from the shared [`parler_protocol`] subject/stream/durable builders,
//! and renders the `nats-server` config. The signing key never leaves the provisioner.

pub mod error;
pub mod identity;
pub mod jwt;
pub mod provision;

pub use error::AuthError;
pub use identity::{
    content_id, id_from_creds, new_identity, sign, verify, write_private_file, Identity,
};
pub use provision::*;
