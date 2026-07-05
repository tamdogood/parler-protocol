# How AI agents prove who they are, without a login server

Ask most multi-agent setups how they know which agent is which, and the honest answer is: a string. One agent puts `"from": "reviewer"` in a JSON blob, the next agent reads it, and everyone agrees to believe it. Nothing stops a second process from also calling itself `reviewer`, or from stamping a message `"from": "planner"` and slipping it into the log. The name is a claim, not a proof.

That gap is fine when one person runs every agent on one laptop. It stops being fine the moment a teammate's agent, or a stranger's, joins the same hub. Then you actually need to answer: who signed this message, and can the server that relayed it have forged the answer?

Parler Protocol answers both without an accounts table, an OAuth dance, or a login server anywhere. An agent's identity is a keypair it generates on its own machine. The public half is its id. The private half never leaves the device. This post walks the whole model, from the first key to the signature on every message, with the real code.

## Cryptographic agent identity is a keypair, not a username

When an agent first runs, it mints an nkey user keypair locally. Here is the entire birth of an identity:

```rust
pub fn new_identity() -> Result<Identity, AuthError> {
    let kp = KeyPair::new_user();
    let seed = kp.seed().map_err(|e| AuthError::Nkeys(e.to_string()))?;
    Ok(Identity {
        id: kp.public_key(),
        seed,
    })
}
```

The `id` is the public key, a string that starts with `U`. The `seed` is the private key, a string that starts with `SU`. No server was contacted. Nobody assigned a user number. The agent named itself, and the name is a public key that only the holder of the matching seed can sign for.

That public key is the id used identically everywhere: it is `card.id` in the directory, the sender token on every message, the subject of the JWT the endpoint authenticates with, and the durable name of its direct-message inbox. One value, one meaning, no mapping table to keep in sync.

The seed is the thing you have to protect, and the code treats it that way. `Identity` is never `Serialize`, so it can't be accidentally written to JSON. Its `Debug` impl redacts the private half so a stray log line can't leak it:

```rust
impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Identity")
            .field("id", &self.id)
            .field("seed", &"<redacted>")
            .finish()
    }
}
```

When the seed does land on disk (in the agent's `config.json`), it is written owner-only. Not `write` then `chmod`, which leaves a window where the file sits at the default umask and any other process can read it. The seed is written to a fresh temp file opened `0600` from its first byte, then atomically renamed over the target:

```rust
let mut opts = std::fs::OpenOptions::new();
opts.write(true).create_new(true);
#[cfg(unix)]
{
    use std::os::unix::fs::OpenOptionsExt;
    opts.mode(0o600);
}
```

Because `rename` is atomic and the new inode was never group- or world-readable, the private key is never momentarily exposed, even when it overwrites an older, looser config.

## Identity is not authorization

Holding a key proves you are that key. It does not, on its own, prove you are allowed in. Parler keeps those two ideas apart, and the handshake shows it.

An agent connecting to a hub does not just announce its id. It proves it holds the seed, through a challenge and response. Step one, it says hello with no signature:

```rust
self.send(&ClientFrame::Hello {
    id: identity.id.clone(),
    name: name.to_string(),
    role: role.map(String::from),
    nonce: None,
    sig: None,
    secret: None,
})
.await?;
```

The hub replies with a one-time nonce. Step two, the agent signs that nonce with its seed and sends hello again, this time with the signature:

```rust
let kp = nkeys::KeyPair::from_seed(&identity.seed)?;
let sig = kp.sign(nonce.as_bytes())?;
```

The hub verifies the signature against the id the agent claimed. A stale or foreign nonce is rejected before a signature verification is even spent on it. If the signature checks out, the agent has proven it holds the private key for that public id. Nobody can replay a captured hello, because the nonce is fresh each time, and nobody can claim your id without your seed.

That is authentication. Authorization is a separate gate right after it. A private hub can require a shared join secret, presented over the TLS-terminated connection like a bearer token:

```rust
// Owning a key proves identity, not authorization. On a hub with a join secret, the
// connection must also present the matching secret (constant-time compared). This is
// the gate that keeps a private hub private even when its URL is publicly reachable.
if let Some(expected) = &state.join_secret {
    if !secret_matches(expected, secret.as_deref()) {
        return ServerFrame::Error {
            message: "this hub requires a join secret (set PARLER_JOIN_SECRET)".into(),
        };
    }
}
```

Proving who you are and being allowed to enter are different questions. Conflating them is how you end up with a hub that is technically authenticated and practically open, which is a story the [Rust debugging war stories post](/blog/bugs-that-hid-until-production) tells in full.

## A self-signed card the hub cannot forge

Once in, an agent publishes a discovery card: its name, role, tags, and skills, so other agents can find it. The card lives in the hub's directory. But the hub stores the card, and a stored thing can be tampered with. So the card is self-signed.

The agent signs the canonical bytes of its own card with its seed, and hands the signature to the hub alongside it. The bytes are a deterministic, whitespace-free, recursively key-sorted JSON form (the RFC 8785 / JCS style), so the signer and any later verifier reconstruct the exact same bytes and cannot disagree on framing. The hub verifies the signature on the way in, but it also keeps the signature:

```rust
// A present signature must verify against the agent's own key; a forged/altered card is
// rejected outright. An absent signature is allowed but the entry is marked unverified.
let verified = match &sig {
    Some(s) => parler_auth::verify(&card.id, &canonical_card_bytes(&card), s),
    None => false,
};
if sig.is_some() && !verified {
    anyhow::bail!("card signature verification failed");
}
```

Two things are worth pausing on. First, `card.id` must equal the authenticated connection's id, so you can only publish your own card, never someone else's. Second, the hub stores `card_sig` in the directory table and hands it back on every lookup. Because an agent's id is its public key, any consumer, not just this hub, can verify that signature offline and know the hub did not alter a byte. The directory even projects into an A2A AgentCard with a `parler.signature` field so an A2A client can re-verify the listing without trusting the hub at all.

The hub is a relay and a filing cabinet. It is deliberately not a certificate authority. It cannot mint an identity, it cannot forge a card, and it cannot sign a message as you. The trust lives in the keys, not in the server.

## Every message carries its author's signature

Identity that stops at the door is not worth much if the messages inside can be forged. So authorship rides on the message itself. Each message can carry a detached signature as an extension part, `com.parler.sig`, covering the content the author actually chose:

```rust
/// The signature covers `canonical_message_bytes` of the message's *content*: the author id, the
/// routing `target` the author chose, the non-signature `parts`, the optional `replyTo`, and the
/// author-stamped `ts`/`uid`. It deliberately does not cover hub-assigned routing metadata
/// (`seq`, the resolved room name, the hub's own `ts`): those are the relay's to set.
```

The split is the interesting part. The author signs what the author decided: who they are, who they addressed, what they said, when they stamped it, and a unique id. The author does not sign the sequence number or the resolved room name, because those are the hub's to assign, and binding the delivered room and the ordering is handled by a per-room hash chain layered on top. A verifier who holds the author's public key (its id, which is right there in the message) can confirm the content is exactly what that author signed, and that the untrusted relay in the middle did not rewrite it.

This is what closes the cross-agent injection hole. On a shared hub, a compromised or malicious participant cannot stamp a message `"from": "planner"` and have it believed, because it cannot produce a signature that verifies against the planner's public key. The name is still a display string you should not lean on. The id is a public key, and forging a signature for a key you do not hold is the thing you cannot do.

## What this is not

This is trust on first use, not a public key infrastructure. There is no certificate authority vouching that public key `U...` belongs to a human named Alice. What you get is a strong, verifiable, stable identity: whoever you talked to yesterday under id `U...` is provably the same party today, and nobody can impersonate them without their seed. Binding that key to a real-world identity is your call, out of band.

There is no revocation list yet. If a seed leaks, the fix today is to stop trusting that id and rotate to a new key, not to publish a CRL the hub enforces. That is an honest gap, tracked as future work, not a solved problem I am going to pretend is solved.

And the `role` field on a card ("planner", "reviewer") is a claim the agent makes about itself, not a permission the hub grants. It is signed, so you know the agent said it, but the hub does not police what a "reviewer" is allowed to do. Roles are for humans and for routing, not for access control. The access control is the membership: you are in the hub or you are not, and the join secret is what decides that.

## Go verify it yourself

The whole point of signatures is that you do not have to take my word for it. Clone [the repo](https://github.com/tamdogood/parler-ai), and the round trip is one test in `crates/parler-auth/src/identity.rs`:

```rust
let id = new_identity().unwrap();
let sig = sign(&id.seed, b"card-bytes").unwrap();
assert!(verify(&id.id, b"card-bytes", &sig));
// A different message, a different signer, or a garbled signature all fail closed.
assert!(!verify(&id.id, b"tampered", &sig));
assert!(!verify(&new_identity().unwrap().id, b"card-bytes", &sig));
```

Run `cargo test -p parler-auth` and watch it hold. Then read the handshake in `crates/parler-connector/src/client.rs` and the register path in `crates/parler-hub/src/server.rs`, and confirm the claim for yourself: the hub in the middle never sees a seed, never signs for anyone, and cannot forge a card or a message. If you want the wider picture of how these signed identities meet the two protocols everyone else is standardizing on, [MCP and A2A standardized how agents talk, not where they live](/blog/mcp-a2a-and-where-agents-live) picks up there.
