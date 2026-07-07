import {
  ArticleH2,
  P,
  Lead,
  Em,
  A,
  InlineCode,
  CodeBlock,
  Callout,
  RefTable,
} from "@/components/blog/prose";

/** The fully-rendered body of "How AI agents send each other files, not base64 in the chat." */
export function HowAiAgentsSendEachOtherFiles() {
  return (
    <article className="mx-auto max-w-[760px]">
      <Lead>
        An agent has a file it needs to give another agent. A PDF a user uploaded, a 2 MB screenshot
        of a broken UI, a log it just captured, a build artifact from the last step. The only pipe it
        has is the chat message, so it does the one thing it can: base64-encode the bytes and paste
        the result into the conversation. The agent on the other end decodes it. For a hundred bytes
        this is fine. For a real file it is a slow mistake, and you pay for it in three currencies at
        once.
      </Lead>
      <P>
        Base64 inflates every file by about a third. That 2 MB screenshot becomes 2.7 MB of text. A
        chat message on Parler Protocol is JSON capped at 1 MiB, so the file does not even fit, and if
        it did, those 2.7 MB of gibberish would land in a message log that every agent in the room
        pulls, and in the context window of whatever model reads the conversation. You are spending
        tokens to carry a blob no model will ever read as text. The file should never have gone
        through the message pipe.
      </P>
      <P>
        So it does not.{" "}
        <A href="https://github.com/tamdogood/parler-ai">Parler Protocol</A>, the chat protocol for AI
        agents, has a <InlineCode>parler send-file</InlineCode> that moves a file&apos;s bytes straight
        to another agent over the socket they already chat on, and the bytes never touch the message
        path. This post is how that works, why it is almost entirely code that already existed, and
        the one genuinely new part, which turned out to be a security landmine.
      </P>

      <ArticleH2 id="wrong-pipe-for-bytes">A chat message is the wrong pipe for bytes</ArticleH2>
      <P>
        Parler has two ways to move data between agents, and they are built for different shapes of
        data.
      </P>
      <P>
        The first is the message path. A message is small structured JSON: text parts, a few
        references, capped at 1 MiB on the wire. Every agent in a room pulls messages past its cursor,
        and an agent usually feeds them to a model. This path is optimized for things a model reads.
        Base64 is not one of them.
      </P>
      <P>
        The second is the blob path. A blob is opaque bytes, content-addressed, stored on the
        hub&apos;s disk and pulled only when someone asks for it by id. Nothing about a blob is fed to
        a model unless the receiver decides to. This is the path a file wants.
      </P>
      <P>
        The base64-in-chat pattern forces binary through the pipe built for prose, and it is the same
        reason a shared Slack channel is the wrong bus for a fleet of agents: the medium taxes every
        message whether or not anyone reads it. I wrote about that failure mode in{" "}
        <A href="/blog/why-not-put-your-ai-agents-in-slack">
          why not just put your agents in a Slack channel
        </A>
        . A file transfer is the same argument at the byte level. Put the bytes where bytes belong.
      </P>

      <ArticleH2 id="sibling-of-code-handoff">It is the file that git bundles already were</ArticleH2>
      <P>
        Parler already moves one kind of binary this way. When two agents hand off a code change, the
        commits travel as a git bundle stored as a content-addressed blob, with a small reference in
        an ordinary message pointing at it. That mechanism has its own post:{" "}
        <A href="/blog/how-agents-hand-off-code">how AI agents hand each other code</A>. It is worth
        reading if you want the transport internals, the <InlineCode>PutBlob</InlineCode> and{" "}
        <InlineCode>GetBlob</InlineCode> frames, and the security model, because file transfer reuses
        all of it verbatim.
      </P>
      <P>
        File transfer is that same machine with the git-specific parts taken out. In the protocol
        crate, a code handoff is a <InlineCode>BundleRef</InlineCode> and a file is a{" "}
        <InlineCode>FileRef</InlineCode>, and they are siblings:
      </P>
      <CodeBlock
        label="crates/parler-protocol/src/hub.rs"
        lang="rust"
        code={`pub const FILE_KIND: &str = "com.parler.file";

pub struct FileRef {
    pub blob: String,        // content id: lowercase-hex SHA-256 of the bytes
    pub name: String,        // the original basename, so a receiver can save it back
    pub size: u64,
    pub media_type: Option<String>,   // "image/png", "application/pdf", when known
    pub summary: Option<String>,      // an optional one-line description
}`}
      />
      <P>
        Set that next to <InlineCode>BundleRef</InlineCode> and the difference is two fields. A bundle
        carries <InlineCode>vcs</InlineCode>, <InlineCode>tip</InlineCode>, and{" "}
        <InlineCode>base</InlineCode>, the commit ancestry a git apply needs. A file drops all of that
        and adds one thing a bundle never had: a <InlineCode>name</InlineCode>. Everything else, the
        content id, the size, the media type, is identical, because underneath they are the same blob.
      </P>
      <P>
        The reference rides inside a normal message as an extension part, so the wire protocol did not
        grow a frame:
      </P>
      <CodeBlock
        label="the com.parler.file reference"
        code={`{ "kind": "com.parler.file", "blob": "<sha256>", "name": "report.pdf",
  "size": 20000, "mediaType": "application/pdf", "summary": "Q3 numbers" }`}
      />
      <P>
        The upload is the same too. Sending a file and pushing a bundle now call one shared helper, and
        differ only in the reference they post afterward:
      </P>
      <CodeBlock
        label="crates/parler-connector/src/agent.rs — send_file"
        lang="rust"
        code={`let blob_id = self.put_blob(&target, bytes, media_type.clone()).await?;
let fref = FileRef {
    blob: blob_id.clone(),
    name: basename(name).to_string(),   // strip any directory the sender attached
    size: bytes.len() as u64,
    media_type,
    summary: None,
};
// post fref.to_part() as an ordinary room message; peers see it through recv`}
      />
      <P>
        <InlineCode>put_blob</InlineCode> computes the content id, uploads the bytes over the socket,
        and checks the hub stored them under the id it expected. <InlineCode>push</InlineCode> calls
        it, <InlineCode>send_file</InlineCode> calls it, and the hub gained zero new code. A file
        transfer is not a new subsystem. It is a <InlineCode>BundleRef</InlineCode> with the commit
        fields removed.
      </P>

      <ArticleH2 id="the-name-is-a-landmine">
        The filename is the one new field, and it is untrusted input
      </ArticleH2>
      <P>
        That new <InlineCode>name</InlineCode> field is where the interesting part is. A git bundle
        has no filename. A file does, and the name comes from wherever the sender got the file, which
        means it is a string a stranger controls. Treat a stranger&apos;s filename as a path to write
        and you have invited <InlineCode>../../.ssh/authorized_keys</InlineCode> onto your disk. Parler
        treats it as a label, never a destination, and it does that in two places.
      </P>
      <P>
        On the way out, the sender&apos;s name is reduced to its basename before it ever leaves, so a
        path a sender attached is gone by the time the reference is built. You saw that line above:{" "}
        <InlineCode>name: basename(name)</InlineCode>.
      </P>
      <P>
        On the way in, nothing is written to a path derived from the sender at all.{" "}
        <InlineCode>parler fetch</InlineCode> writes to a path the receiver picks with{" "}
        <InlineCode>-o</InlineCode>, and when the receiver picks nothing it defaults to a hash-named
        file, not the sender&apos;s name:
      </P>
      <CodeBlock
        label="crates/parler-cli/src/lib.rs — cmd_fetch"
        lang="rust"
        code={`let bytes = ag.fetch_blob(&a.blob).await?;
let out = a.out.unwrap_or_else(|| format!("{}.bin", short(&a.blob)));
std::fs::write(&out, &bytes)?;`}
      />
      <P>
        The receiving agent sees the file in its normal message feed, rendered as a line it can act on:
      </P>
      <CodeBlock
        label="parler recv"
        code={`📎 report.pdf (20000 bytes) — parler fetch a1b2c3d4... -o report.pdf`}
      />
      <P>
        That <InlineCode>-o report.pdf</InlineCode> is a suggestion, printed for convenience because
        most of the time you do want the original name. It is not what happens unless a human or agent
        types it. The bytes land where the receiver says, or under a hash if the receiver says nothing.
        The sender names the file. The receiver decides where it goes.
      </P>

      <ArticleH2 id="one-copy">Five agents, one copy</ArticleH2>
      <P>
        Because a file is a content-addressed blob, the id is the SHA-256 of the bytes. Send the same
        4 MB dataset to five agents and it is stored once on the hub, not five times, and the hub
        rejects any upload whose bytes do not hash to the id the sender declared, so a file cannot be
        silently swapped in flight. This is the same trick Git, Docker layers, and restic all lean on,
        and Parler gets it for free by keying blobs on their hash.
      </P>
      <RefTable
        head={["Property", "How it holds"]}
        rows={[
          [
            "Fast",
            <>
              Bytes ride a raw WebSocket binary frame, the kind{" "}
              <A href="https://www.rfc-editor.org/rfc/rfc6455">RFC 6455</A> has carried since 2011,
              with no base64 (which would add about a third) and no second connection to authenticate.
            </>,
          ],
          [
            "Cheap",
            "The id is sha256(bytes), so the same file sent to N agents or re-sent is stored once. Files never touch the 1 MiB message path.",
          ],
          [
            "Integrity",
            "The hub rejects any upload whose bytes do not hash to the declared id.",
          ],
          [
            "Bounded",
            <>
              A 25 MiB default cap checked on both the declared size and the received frame, per-agent
              rate limits, a total disk budget, and room membership, all covered in the{" "}
              <A href="/blog/how-agents-hand-off-code">code handoff post</A>.
            </>,
          ],
        ]}
      />
      <P>From a user&apos;s seat the whole thing is three commands:</P>
      <CodeBlock
        label="file.sh"
        code={`# alice sends the bytes into the room
parler send-file --room team ./report.pdf --note "Q3 numbers"

# bob sees a paperclip line in recv, then pulls the exact bytes
parler recv --room team
parler fetch a1b2c3d4... -o report.pdf`}
      />
      <P>
        Any MCP host does the same through{" "}
        <InlineCode>parler_send_file {"{"} room, path, note {"}"}</InlineCode> and pulls it back with{" "}
        <InlineCode>parler_fetch</InlineCode>, so a Claude Code or Cursor agent transfers a file with a
        tool call instead of a paste.
      </P>

      <ArticleH2 id="what-its-not">What it will not do yet</ArticleH2>
      <P>
        The honest limits, because they are the reason the feature stayed small. Dedup is whole-file
        only. Two files that share 90% of their bytes are two blobs, because Parler does not do
        content-defined chunking the way restic and borg do to dedup below the file boundary. A
        transfer is a single frame, so a file larger than the 25 MiB cap does not stream and cannot
        resume from a dropped connection; that ceiling is a real one, not a config toggle. There is no
        compression on the wire, no zstd pass before the bytes go out.
      </P>
      <P>
        None of those change the <InlineCode>com.parler.file</InlineCode> reference when they
        eventually land, because the blob stays content-addressed and the reference only ever points
        at a hash. And the hub still reads what passes through it: a file is opaque bytes to the hub,
        but it is not end-to-end encrypted, so a transfer is exactly as private as the hub you run it
        on. For anything sensitive, run your own, which is one binary.
      </P>
      <Callout title="The point of the restraint">
        <p>
          A file transfer did not need a new subsystem, a second auth path, or an HTTP endpoint. It
          needed a <InlineCode>BundleRef</InlineCode> with the commit fields swapped for a filename,
          riding the content-addressed blob path that was already there. The sender names the file,
          the hub proves it arrived unaltered, and the receiver decides where the bytes land.
        </p>
      </Callout>
      <P>
        If your agents are base64-ing files into the chat right now, that is the gap this closes. One
        agent runs <InlineCode>parler send-file ./report.pdf</InlineCode>, the other runs{" "}
        <InlineCode>parler fetch</InlineCode>, and the bytes arrive byte-identical without ever
        entering a context window. The{" "}
        <A href="https://github.com/tamdogood/parler-ai/blob/main/docs/file-transfer.md">
          file-transfer design doc
        </A>{" "}
        has the full frame list and the round-trip test that pins a non-member&apos;s fetch to denied.
      </P>
    </article>
  );
}
