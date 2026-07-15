# How AI agents hand each other code, not just words

Two AI agents can talk all day. One can describe a fix, paste a diff into the chat, explain which files it touched and why. The other agent reads that and tries to reconstruct the change on its own machine. If you have ever watched this happen you know how it goes. The diff is truncated. A file path is slightly wrong. The base the patch assumed is three commits behind. The receiving agent applies its best guess and now the two repos have quietly diverged.

The problem is that a chat protocol moves words, and a code change is not words. It is a set of commits with ancestry. It has a base it expects you to already have. It is either applied exactly or it is wrong. So when I built the code-handoff layer into Parler Protocol, the chat protocol for AI agents, the question was not "how do we format the diff nicely." It was "how do we move the actual change, byte for byte, so the receiver ends up with the exact commits the sender had, and nothing gets reconstructed from a description."

The answer turned out to be a git bundle carried as a content-addressed blob over the socket the agents already chat on. No new service, no second auth path, and no GitHub-in-a-box. This post is how that works, and the handful of decisions that kept it small.

## Talking about a change versus handing it over

Before the handoff layer, two Parler Protocol agents had exactly two ways to share a change, and both were lossy.

They could send it as a chat message. That is fine for "I bumped the timeout to 30 seconds," and useless for a five-commit branch. Text has no ancestry.

Or they could write it as a memory fact, the same durable key-value notes agents leave each other. Better for structured state, still not a patch. A fact is a string.

What was missing was a way to say: here is the change itself, take these commits, they hash to exactly this, apply them and you have what I have. That is an artifact handoff, and it needs a primitive that neither chat nor memory gives you.

## The one decision: split the bytes from the reference

The whole design rests on one split. A handoff is two separate things:

The **blob** is the bundle bytes. The hub stores them content-addressed, which means the id of a blob is the SHA-256 of its bytes. Store it under its own hash and three things fall out for free: identical bundles dedupe, tampering is detectable because altered bytes no longer match their id, and the hub never has to understand what is inside. To the hub a bundle is opaque. It never runs git.

The **reference** is an ordinary room message that points at the blob. It rides the exact machinery Parler Protocol already had for chat. There is a first-class extension part on the wire, so the reference is just a message part of a known kind:

```json
{ "blob": "<sha256>", "vcs": "git", "tip": "<commit>", "base": "<base commit or null>",
  "summary": "feat: add X", "size": 12345, "mediaType": "application/x-git-bundle" }
```

Because the reference is an ordinary message, everything Parler Protocol already does for messages works unchanged. Send and receive are the same calls. The per-room cursor tracks it. Durability persists it. Reconnect-resume replays it. The Stop-hook that wakes a sleeping agent fires on it. And an old client that has never heard of a bundle still sees a renderable extension part, so it degrades to `[bundle: feat: add X]` instead of crashing.

In the protocol crate this is a small struct with a round-trip to and from a message part:

```rust
pub const BUNDLE_KIND: &str = "com.parler.bundle";

pub struct BundleRef {
    pub blob: String,        // content id: lowercase-hex SHA-256 of the bytes
    pub vcs: String,         // "git", or later "patch", "tar", ...
    pub tip: Option<String>,
    pub base: Option<String>,
    pub summary: Option<String>,
    pub size: u64,
    pub media_type: Option<String>,
}

impl BundleRef {
    pub fn to_part(&self) -> Part { /* serialize to a Part::Extension */ }
    pub fn from_part(part: &Part) -> Option<BundleRef> { /* parse it back */ }
}
```

That is the entire protocol surface for the reference. No new frame, no version bump, no schema migration. The extension part was already forward-compatible, so a handoff is a message that some clients understand more deeply than others.

## Why a git bundle, and not a diff or a tarball

A git bundle is a single file that carries commits and their ancestry. You can build a full one that carries a branch back to its root, or a thin one that carries only `base..HEAD` and expects the receiver to already have the base. No live git server sits between the two sides. The sender runs one command, the receiver runs one command, and the objects move as a file in between.

Building it is a shell out to git, nothing clever:

```rust
// tip = git rev-parse <ref>; summary = git log -1 --format=%s <ref>
let range = match base {
    Some(b) => format!("{b}..{gitref}"),   // a thin patch series
    None => gitref.to_string(),            // full history to the tip
};
git_in(repo, &["bundle", "create", tmp_path, &range])?;
let bytes = std::fs::read(&tmp)?;
```

The `vcs` and `mediaType` fields on the reference are there so this can grow to carry a plain patch or a tarball later without changing the format. But a git bundle is the first-class case because it is the one that preserves exactly what a coding agent cares about: the commits, in order, with their real hashes.

## Transport: reuse the socket, don't open a second one

The reference project I borrowed the idea from shipped bytes over HTTP: a `POST` to push, a `GET` to fetch, a separate auth story for each. Parler Protocol does not, and the reason is worth stating because it is the kind of decision that keeps a system small.

The WebSocket the agents chat on is already authenticated. An agent proved who it was with an nkey challenge-response when it connected, and that connection already supports binary frames, they were just being ignored. So the bytes ride that. What you get by not opening a second channel:

- No new dependency. There is no HTTP client in the connector, nothing to pull in, nothing to keep patched.
- No second auth path. Authorization is room membership on a socket whose identity is already proven. There is no capability-token table to mint, expire, and revoke, which the directory tokens needed and this deliberately does not.
- One code path for one thing.

An upload is one request and one binary frame:

```
client -> PutBlob { target, sha256, size, mediaType }
hub    -> BlobReady { id }              # you're a member and the size is ok; send the bytes
client -> <binary frame: the bundle>    # the whole blob, one frame, capped at max_blob_bytes
hub    -> BlobStored { id }             # verified sha256(bytes) == id and len == size
```

A download is the mirror of that:

```
client -> GetBlob { id }                # hub checks you're a member of a room the blob is in
hub    -> BlobIncoming { id, size }
hub    -> <binary frame: the bundle>
```

The handoff message itself still goes out with the ordinary send and is read with the ordinary receive. Only the blob movement is new, and it is the only place the socket loop grows past pure request-and-reply: after a `PutBlob` is acked, the connection is holding one slot open for exactly one incoming blob, and the very next binary frame is consumed as those bytes. Any other frame while that slot is open is an error. That is the whole extension to the loop, and it is bounded on purpose: single frame, size capped.

## On the receiving end: recv, fetch, apply

From the other agent's seat, a handoff shows up in its normal message feed. The receive command renders the bundle part as a line it can act on:

```
📦 feat: add retry backoff (a1b2c3, 12408 bytes) — parler apply a1b2c3d4e5f6...
```

Two verbs follow. `parler fetch <id>` pulls the bytes and writes the `.bundle` file, nothing more. `parler apply <id>` is the one that touches a repo, and how it touches it is the most deliberate part of the whole feature:

```rust
git_in(None, &["bundle", "verify", tmp])?;   // reject if the base it's thin against is missing
git_in(None, &["fetch", tmp])?;              // import the objects, working tree untouched
git_in(None, &["bundle", "list-heads", tmp])?;
git_in(None, &["update-ref", &refname, &tip_sha])?;  // pin the tip under refs/parler/<id>
```

Apply imports the commits and pins them under a namespaced ref like `refs/parler/a1b2c3`. It never merges. It never checks out. Your working tree is exactly as you left it, and the imported work is sitting in a ref you can inspect with `git log refs/parler/a1b2c3` and merge with `git merge refs/parler/a1b2c3` when you have looked at it. The CLI exposes this as `parler apply`; MCP hosts can call `parler_apply` with an explicit repository path. Both stop at the isolated ref. Merging or checking out remains a separate human action.

## The security model, such as it is

The nice thing about building on content-addressing and an existing membership model is that most of the security story is inherited, not invented.

| Property | How it holds |
|---|---|
| Integrity | The id is the SHA-256 of the bytes. The hub rejects any blob whose bytes do not hash to the declared id, so you cannot store something under a hash it does not match. |
| Authorization | A blob is bound to the rooms it was posted to. Only a member of one of those rooms can fetch it. That is the same `is_member` check that gates messages, no new ACL concept. |
| No new attack surface | Bytes ride the already-authenticated socket. There is no HTTP endpoint to harden and no capability token to leak. |
| The hub never executes | The bundle is opaque bytes to the hub. There is no git on the server, so there is no server-side git to exploit. |
| Apply is isolated | CLI and MCP apply import into a side ref and never merge, check out, or modify the working tree. |

Membership is checked at fetch time against every room the blob is bound to, because the same content-addressed bytes can be handed off in more than one room. If you are a member of any room the blob lives in, you can read it. If you are a member of none, the fetch is denied. That last part is one of the things the end-to-end test pins down: a non-member's fetch returns denied, not bytes.

Bounding is the other half. A blob is capped at 25 MiB by default, enforced both when `PutBlob` declares its size and again on the received frame, so a lie about the size does not get you a bigger write. Beyond that there are per-agent rate limits, because the first thing you want the moment a hub is public is a ceiling on how much one agent can push.

## What this deliberately is not

It would have been easy to let this grow into a GitHub replacement. The project I borrowed from has one: a server-side commit graph with lineage and diffs, browsable in a UI. I took the transport and left the metaphor.

There is no bare repo on the server, no commit DAG, no lineage or diff endpoints. There is no web UI for code; the website stays a read-only directory browser. There is no auto-merge into anyone's working tree. All of the git semantics live on the agents' own machines, where git already is, and the hub's entire job is to move an opaque file from one member of a room to another and prove it arrived unaltered.

That restraint is the point. A handoff did not need a new subsystem. It needed one honest primitive, content-addressed bytes with a message pointing at them, riding the machinery that was already there. The reference is a chat message. The bytes are a blob. The receiver ends up with the exact commits the sender had, because nothing along the way ever tried to reconstruct them from a description.

If two of your agents are still pasting diffs at each other, that is the gap this closes. Point them at a hub, `parler push` from one, `parler apply` on the other, and the change moves as a change.
