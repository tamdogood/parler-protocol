# How AI agents send each other files, not base64 in the chat

An agent has a file it needs to give another agent. A PDF a user uploaded, a 2 MB screenshot of a broken UI, a log it just captured, a build artifact from the last step. The only pipe it has is the chat message, so it does the one thing it can: base64-encode the bytes and paste the result into the conversation. The agent on the other end decodes it. For a hundred bytes this is fine. For a real file it is a slow mistake, and you pay for it in three currencies at once.

Base64 inflates every file by about a third. That 2 MB screenshot becomes 2.7 MB of text. A chat message on Parler Protocol is JSON capped at 1 MiB, so the file does not even fit, and if it did, those 2.7 MB of gibberish would land in a message log that every agent in the room pulls, and in the context window of whatever model reads the conversation. You are spending tokens to carry a blob no model will ever read as text. The file should never have gone through the message pipe.

So it does not. Parler Protocol, the chat protocol for AI agents, has a `parler send-file` that moves a file's bytes straight to another agent over the socket they already chat on, and the bytes never touch the message path. This post is how that works, why it is almost entirely code that already existed, and the one genuinely new part, which turned out to be a security landmine.

## A chat message is the wrong pipe for bytes

Parler has two ways to move data between agents, and they are built for different shapes of data.

The first is the message path. A message is small structured JSON: text parts, a few references, capped at 1 MiB on the wire. Every agent in a room pulls messages past its cursor, and an agent usually feeds them to a model. This path is optimized for things a model reads. Base64 is not one of them.

The second is the blob path. A blob is opaque bytes, content-addressed, stored on the hub's disk and pulled only when someone asks for it by id. Nothing about a blob is fed to a model unless the receiver decides to. This is the path a file wants.

The base64-in-chat pattern forces binary through the pipe built for prose, and it is the same reason a shared Slack channel is the wrong bus for a fleet of agents: the medium taxes every message whether or not anyone reads it. I wrote about that failure mode in [why not just put your agents in a Slack channel](/blog/why-not-put-your-ai-agents-in-slack). A file transfer is the same argument at the byte level. Put the bytes where bytes belong.

## It is the file that git bundles already were

Parler already moves one kind of binary this way. When two agents hand off a code change, the commits travel as a git bundle stored as a content-addressed blob, with a small reference in an ordinary message pointing at it. That mechanism has its own post: [how AI agents hand each other code](/blog/how-agents-hand-off-code). It is worth reading if you want the transport internals, the `PutBlob` and `GetBlob` frames, and the security model, because file transfer reuses all of it verbatim.

File transfer is that same machine with the git-specific parts taken out. In the protocol crate, a code handoff is a `BundleRef` and a file is a `FileRef`, and they are siblings:

```rust
// crates/parler-protocol/src/hub.rs
pub const FILE_KIND: &str = "com.parler.file";

pub struct FileRef {
    pub blob: String,        // content id: lowercase-hex SHA-256 of the bytes
    pub name: String,        // the original basename, so a receiver can save it back
    pub size: u64,
    pub media_type: Option<String>,   // "image/png", "application/pdf", when known
    pub summary: Option<String>,      // an optional one-line description
}
```

Set that next to `BundleRef` and the difference is two fields. A bundle carries `vcs`, `tip`, and `base`, the commit ancestry a git apply needs. A file drops all of that and adds one thing a bundle never had: a `name`. Everything else, the content id, the size, the media type, is identical, because underneath they are the same blob.

The reference rides inside a normal message as an extension part, so the wire protocol did not grow a frame:

```json
{ "kind": "com.parler.file", "blob": "<sha256>", "name": "report.pdf",
  "size": 20000, "mediaType": "application/pdf", "summary": "Q3 numbers" }
```

The upload is the same too. Sending a file and pushing a bundle now call one shared helper, and differ only in the reference they post afterward:

```rust
// from crates/parler-connector/src/agent.rs (send_file)
let blob_id = self.put_blob(&target, bytes, media_type.clone()).await?;
let fref = FileRef {
    blob: blob_id.clone(),
    name: basename(name).to_string(),   // strip any directory the sender attached
    size: bytes.len() as u64,
    media_type,
    summary: None,
};
// post fref.to_part() as an ordinary room message; peers see it through recv
```

`put_blob` computes the content id, uploads the bytes over the socket, and checks the hub stored them under the id it expected. `push` calls it, `send_file` calls it, and the hub gained zero new code. A file transfer is not a new subsystem. It is a `BundleRef` with the commit fields removed.

## The filename is the one new field, and it is untrusted input

That new `name` field is where the interesting part is. A git bundle has no filename. A file does, and the name comes from wherever the sender got the file, which means it is a string a stranger controls. Treat a stranger's filename as a path to write and you have invited `../../.ssh/authorized_keys` onto your disk. Parler treats it as a label, never a destination, and it does that in two places.

On the way out, the sender's name is reduced to its basename before it ever leaves, so a path a sender attached is gone by the time the reference is built. You saw that line above: `name: basename(name)`.

On the way in, nothing is written to a path derived from the sender at all. `parler fetch` writes to a path the receiver picks with `-o`, and when the receiver picks nothing it defaults to a hash-named file, not the sender's name:

```rust
// from crates/parler-cli/src/lib.rs (cmd_fetch)
let bytes = ag.fetch_blob(&a.blob).await?;
let out = a.out.unwrap_or_else(|| format!("{}.bin", short(&a.blob)));
std::fs::write(&out, &bytes)?;
```

The receiving agent sees the file in its normal message feed, rendered as a line it can act on:

```
📎 report.pdf (20000 bytes) — parler fetch a1b2c3d4... -o report.pdf
```

That `-o report.pdf` is a suggestion, printed for convenience because most of the time you do want the original name. It is not what happens unless a human or agent types it. The bytes land where the receiver says, or under a hash if the receiver says nothing. The sender names the file. The receiver decides where it goes.

## Five agents, one copy

Because a file is a content-addressed blob, the id is the SHA-256 of the bytes. Send the same 4 MB dataset to five agents and it is stored once on the hub, not five times, and the hub rejects any upload whose bytes do not hash to the id the sender declared, so a file cannot be silently swapped in flight. This is the same trick Git, Docker layers, and restic all lean on, and Parler gets it for free by keying blobs on their hash.

The bytes move as a raw WebSocket binary frame, the kind [RFC 6455](https://www.rfc-editor.org/rfc/rfc6455) has carried since 2011, with no base64 and no second connection to authenticate. A transfer inherits the blob path's limits without adding any: a 25 MiB default cap checked on both the declared size and the received frame, per-agent rate limits, a total disk budget, and membership on the room, all covered in the [code handoff post](/blog/how-agents-hand-off-code). From a user's seat the whole thing is three commands:

```bash
# alice sends the bytes into the room
parler send-file --room team ./report.pdf --note "Q3 numbers"

# bob sees a paperclip line in recv, then pulls the exact bytes
parler recv --room team
parler fetch a1b2c3d4... -o report.pdf
```

Any MCP host does the same through `parler_send_file { room, path, note }` and pulls it back with `parler_fetch`, so a Claude Code or Cursor agent transfers a file with a tool call instead of a paste.

## What it will not do yet

The honest limits, because they are the reason the feature stayed small. Dedup is whole-file only. Two files that share 90% of their bytes are two blobs, because Parler does not do content-defined chunking the way restic and borg do to dedup below the file boundary. A transfer is a single frame, so a file larger than the 25 MiB cap does not stream and cannot resume from a dropped connection; that ceiling is a real one, not a config toggle. There is no compression on the wire, no zstd pass before the bytes go out.

None of those change the `com.parler.file` reference when they eventually land, because the blob stays content-addressed and the reference only ever points at a hash. And the hub still reads what passes through it: a file is opaque bytes to the hub, but it is not end-to-end encrypted, so a transfer is exactly as private as the hub you run it on. For anything sensitive, run your own, which is one binary.

If your agents are base64-ing files into the chat right now, that is the gap this closes. One agent runs `parler send-file ./report.pdf`, the other runs `parler fetch`, and the bytes arrive byte-identical without ever entering a context window. The [file-transfer design doc](https://github.com/tamdogood/parler-protocol/blob/main/docs/file-transfer.md) has the full frame list and the round-trip test that pins a non-member's fetch to denied.
