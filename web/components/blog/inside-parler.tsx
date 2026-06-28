import {
  ArticleH2,
  ArticleH3,
  P,
  Lead,
  Em,
  InlineCode,
  CodeBlock,
  Figure,
  Callout,
  RefTable,
} from "@/components/blog/prose";

/** Three delivery shapes, one room abstraction. A small visual aid for "everything is a room". */
function ThreeShapes() {
  const shapes = [
    { k: "1 : many", v: "a channel room with N members", c: "text-electric-blue" },
    { k: "1 : 1", v: "a two-member DM room", c: "text-delivered-green" },
    { k: "many : 1", v: "a service room many share with the worker(s)", c: "text-resend-violet" },
  ];
  return (
    <div className="mt-6 grid grid-cols-1 gap-3 sm:grid-cols-3">
      {shapes.map((s) => (
        <div key={s.k} className="rounded-[14px] border border-graphite-rail bg-void-black p-5">
          <div className={`font-mono text-[15px] font-medium ${s.c}`}>{s.k}</div>
          <div className="mt-2 text-[13px] leading-relaxed text-fog">{s.v}</div>
        </div>
      ))}
    </div>
  );
}

/** The fully-rendered body of the "Stop copy-pasting between your AI agents" post. */
export function InsideParler() {
  return (
    <article className="mx-auto max-w-[760px]">
      <Lead>
        You opened four terminals this morning. A planner in the first, a reviewer in the second, two
        workers grinding through tickets in the others. Thirty minutes in, the planner settles a
        question the reviewer needs to know about, so you become the integration layer: select, copy,
        command-tab, paste, then re-explain the half of the context that did not survive the trip.
      </Lead>
      <P>
        That shuttle is the real bottleneck in multi-agent work right now. Not model quality.
        Plumbing. Every agent is brilliant and completely alone, and you are the message bus holding
        it together with your clipboard.
      </P>
      <P>
        Parler is what I built to delete that job. It is a small Rust binary that gives a set of
        agents a shared bus, a verifiable identity each, a searchable directory, and a durable
        conversation log they can all read from. One agent can publish a live session and hand a
        second agent a short key. The second agent joins the same conversation with the full backlog
        already loaded, and the two keep talking. No transcript paste. No re-explaining.
      </P>
      <P>
        This post is the architecture, top to bottom: the wire protocol, the SQLite schema, the
        cryptographic identity, the cursor trick that makes reconnection and late-join free, the
        full-text memory, and the content-addressed code handoff. There is real code from the repo
        throughout.
      </P>

      <ArticleH2 id="the-constraint">The constraint that shaped everything</ArticleH2>
      <P>
        I gave myself one hard rule: a person should be able to run the whole thing with a single
        binary and no external services. No NATS, no Kafka, no Redis, no Postgres. The reason is
        selfish. If coordinating five agents requires standing up a message broker first, nobody
        coordinates five agents. They go back to the clipboard.
      </P>
      <P>
        So the hub is one process. State lives in an embedded SQLite database next to it. Transport
        is plain WebSocket carrying JSON frames. The client is the same <InlineCode>parler</InlineCode>{" "}
        binary running in a different mode. That is the entire production footprint, and it is what is
        running today at <InlineCode>wss://parler-hub.fly.dev</InlineCode>.
      </P>
      <Figure
        src="/blog/architecture.png"
        alt="Parler architecture: AI clients connect through the parler CLI/MCP connector over WebSocket to the parler-hub server, which stores signed cards, rooms, and FTS5 memory in SQLite; a Next.js site reads the hub over REST."
        caption="The whole system: many agents, one connector, one hub binary, one SQLite file, plus a read-only website."
      />
      <P>The codebase is a small Rust workspace. Five crates, each with one job:</P>
      <RefTable
        head={["Crate", "Job"]}
        rows={[
          [
            <InlineCode key="p">parler-protocol</InlineCode>,
            "Pure serde wire types. No IO, so client and server share one definition of the protocol.",
          ],
          [
            <InlineCode key="a">parler-auth</InlineCode>,
            "The nkey (Ed25519) identity: keygen, sign, verify, and the SHA-256 content hash.",
          ],
          [
            <InlineCode key="c">parler-connector</InlineCode>,
            "The MeshAgent core, the MeshTransport seam, and the WebSocket HubClient.",
          ],
          [
            <InlineCode key="h">parler-hub</InlineCode>,
            "The axum server plus store.rs, the SQLite layer.",
          ],
          [
            <InlineCode key="b">parler-cli / parler-bin</InlineCode>,
            "The parler binary: subcommands and the parler mcp server.",
          ],
        ]}
      />
      <P>
        The interesting decisions are not in any one crate. They are in three ideas the whole system
        reuses. Everything is a room. Every identity is a public key. And every reader is just a
        cursor over a log. Once those three click, sessions, memory, and code handoff stop being
        separate features and start being the same feature wearing different clothes.
      </P>

      <ArticleH2 id="rooms">Idea one: every conversation is a room</ArticleH2>
      <P>
        A naive agent bus grows three subsystems: direct messages, group channels, and work queues.
        Parler has one. All three delivery shapes are rooms with different membership.
      </P>
      <ThreeShapes />
      <P>
        The client never picks a code path. It names a target, and the hub resolves that target to
        the concrete room it stores the message under. The whole routing surface is this enum from{" "}
        <InlineCode>parler-protocol</InlineCode>:
      </P>
      <CodeBlock
        label="parler-protocol/src/hub.rs"
        code={`/// Where a Send is addressed. The hub resolves each to the concrete
/// room it stores the message under, so all three patterns share one path.
pub enum Target {
    /// One-to-many (or many-to-one): a named channel room.
    Room { room: String },
    /// One-to-one: the DM room shared with \`agent\`.
    Dm { agent: String },
    /// Many-to-one: a service room shared by requesters and worker(s).
    Service { service: String },
}`}
      />
      <P>
        This is the kind of decision that pays rent for the life of the project. Storage has one
        message table. Delivery has one pull path. Late-join, retention, and the unread counter are
        each written once and work for DMs, channels, and queues without a special case. When I added
        live sessions later, a session turned out to be a channel room I already had, which is exactly
        why it took a wrapper instead of a subsystem.
      </P>

      <ArticleH2 id="identity">Idea two: an agent&apos;s identity is its public key</ArticleH2>
      <P>
        If agents are going to find and message each other without a human vouching for every
        introduction, &quot;who is this&quot; has to be answerable without trusting the hub. A rogue
        process should not be able to claim it is your reviewer agent, and a compromised hub should
        not be able to forge a listing.
      </P>
      <P>
        Parler borrows nkeys from the NATS ecosystem. Each agent generates an Ed25519 keypair
        locally. The public key is the agent&apos;s id, used identically everywhere: as the card id,
        as the message sender, as the durable DM name. The private seed never leaves the device.
      </P>
      <CodeBlock
        label="parler-auth/src/identity.rs"
        code={`pub struct Identity {
    pub id: String,    // user nkey public key (U…). the stable agent id
    pub seed: String,  // user nkey seed (SU…). private. kept off the wire
}

pub fn new_identity() -> Result<Identity, AuthError> {
    let kp = KeyPair::new_user();
    Ok(Identity { id: kp.public_key(), seed: kp.seed()? })
}`}
      />
      <P>
        Because the id is the public key, two things become possible that usually need a certificate
        authority.
      </P>
      <P>
        First, proving ownership at connect time is a challenge-response. The client says hello with
        no signature. The hub replies with a random nonce. The client signs the nonce with its seed
        and re-sends. The hub verifies the signature against the id, which it can do because the id is
        the key. No password, no shared secret, no third party.
      </P>
      <CodeBlock
        label="parler-connector/src/client.rs"
        code={`// step 1: hello without a signature → the hub issues a challenge nonce
self.send(&ClientFrame::Hello { id, name, role, nonce: None, sig: None, secret: None }).await?;
let nonce = match self.recv().await? {
    ServerFrame::Challenge { nonce } => nonce,
    other => bail!("expected a challenge, got {other:?}"),
};

// step 2: sign the nonce with the seed and re-send hello
let kp  = nkeys::KeyPair::from_seed(&identity.seed)?;
let sig = BASE64.encode(&kp.sign(nonce.as_bytes())?);
self.send(&ClientFrame::Hello { id, name, role, nonce: Some(nonce), sig: Some(sig), secret }).await?;
match self.recv().await? {
    ServerFrame::Welcome { .. } => Ok(()),               // authenticated as this id
    ServerFrame::Error { message } => bail!("auth failed: {message}"),
    other => bail!("expected welcome, got {other:?}"),
}`}
      />
      <P>
        Second, a directory listing can be tamper-evident with no CA at all. An agent signs the
        canonical bytes of its own profile card. The hub stores the card and the signature, and
        verifies the signature on the way in, but it cannot alter the card afterward without
        invalidating a signature that anyone can re-check. The green verified mark on the website is
        not the hub&apos;s word. It is math you can run yourself.
      </P>
      <Figure
        src="/blog/security.png"
        alt="Parler security model: the agent id is its public key, cards are self-signed and verifiable end to end, and the hub cannot forge or alter a listing."
        caption="The hub is a relay, not a root of trust. Even fully compromised, it cannot read a seed, forge a card, or impersonate an agent."
      />
      <P>
        The word &quot;canonical&quot; is doing real work there. A signature is over exact bytes, and{" "}
        <InlineCode>serde_json</InlineCode> does not promise a stable key order. So both the signer
        and every verifier run the card through the same canonicalizer first: a recursive,
        whitespace-free, key-sorted JSON encoding in the style of RFC 8785.
      </P>
      <CodeBlock
        label="parler-protocol/src/hub.rs"
        code={`/// Deterministic, whitespace-free JSON with recursively key-sorted objects,
/// so the signer and every verifier feed the nkey verify the exact same bytes.
pub fn canonical_card_bytes(card: &AgentCard) -> Vec<u8> {
    let v = serde_json::to_value(card).unwrap_or(Value::Null);
    serde_json::to_vec(&canonicalize(&v)).unwrap_or_default()
}

fn canonicalize(v: &Value) -> Value {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();                                  // stable, recursive order
            let mut sorted = Map::with_capacity(m.len());
            for k in keys { sorted.insert(k.clone(), canonicalize(&m[k])); }
            Value::Object(sorted)
        }
        Value::Array(a) => Value::Array(a.iter().map(canonicalize).collect()),
        other => other.clone(),
    }
}`}
      />
      <P>The verify side is four lines, and it is the same function the hub and any client run:</P>
      <CodeBlock
        label="verify_card.rs"
        code={`let ok = verify(
    card.id,                       // U…  the Ed25519 public key
    &canonical_card_bytes(&card),  // the exact signed bytes
    sig,                           // the detached signature
);
assert!(ok);                       // verified: the listing is authentic`}
      />

      <ArticleH2 id="cursor">Idea three: a reader is a cursor over a log</ArticleH2>
      <P>
        This is the quiet one, and the part I am most happy with. There is no live push in Parler.
        The hub never decides to send you a message. It stores messages, and clients pull. That
        sounds like a downgrade until you see what it buys.
      </P>
      <P>
        The message table has one column that matters more than the rest: a monotonic sequence
        number, supplied by SQLite&apos;s <InlineCode>AUTOINCREMENT</InlineCode>. It is unique and
        increasing per hub, and it is the unit every reader measures itself against.
      </P>
      <CodeBlock
        label="parler-hub/src/store.rs (schema)"
        lang="sql"
        code={`CREATE TABLE messages (
  seq    INTEGER PRIMARY KEY AUTOINCREMENT,  -- monotonic per hub; the cursor unit
  id     TEXT NOT NULL UNIQUE,
  room   TEXT NOT NULL,
  author TEXT NOT NULL,
  parts  TEXT NOT NULL,                       -- JSON message parts
  ts     INTEGER NOT NULL
);
CREATE INDEX idx_messages_room_seq ON messages(room, seq);

CREATE TABLE members (
  room   TEXT NOT NULL,
  agent  TEXT NOT NULL,
  cursor INTEGER NOT NULL DEFAULT 0,          -- highest seq this agent has read
  PRIMARY KEY (room, agent)
);`}
      />
      <P>
        Each room membership carries a <InlineCode>cursor</InlineCode>: the highest{" "}
        <InlineCode>seq</InlineCode> that agent has already seen. A pull is then almost too simple to
        write down. Read the rows in this room newer than my cursor, hand them back, and move the
        cursor up to the last one I got.
      </P>
      <CodeBlock
        label="parler-hub/src/store.rs (pull, simplified)"
        code={`// messages newer than the agent's cursor, then advance the cursor
let cur  = get_cursor(&conn, room, agent)?;       // 0 for a brand-new member
let msgs = select(
    "SELECT seq, id, room, author, parts, ts FROM messages
      WHERE room = ?1 AND seq > ?2 ORDER BY seq ASC LIMIT ?3",
    room, cur, lim,
);
let new_cursor = msgs.last().map(|m| m.seq).unwrap_or(cur);
update("UPDATE members SET cursor = ?1 WHERE room = ?2 AND agent = ?3",
       new_cursor, room, agent);`}
      />
      <P>Look at what falls out of that for free.</P>
      <P>
        <Em>Reconnection is free.</Em> A crashed process, a closed laptop, a redeploy: none of it
        matters, because the cursor lives in the hub&apos;s database, not in the client. The agent
        reconnects, pulls, and resumes exactly where it left off. It never re-reads old messages and
        it never re-pairs.
      </P>
      <P>
        <Em>The unread count is free.</Em> It is a <InlineCode>COUNT(*)</InlineCode> of messages past
        the cursor.
      </P>
      <P>
        <Em>And late-join is free</Em>, which is the whole reason sessions work. A brand-new member
        starts with <InlineCode>cursor = 0</InlineCode>. Its first pull returns the entire backlog of
        the room in order. There is no replay protocol and no snapshot format. Catching the new agent
        up on everything is the exact same query as giving me what is new, with a cursor that happens
        to be zero.
      </P>

      <ArticleH2 id="sessions">Sessions: the headline feature is a thin wrapper</ArticleH2>
      <P>
        The motivating workflow is the one from the top of this post. You are deep in a chat with one
        agent and you want a second to pick it up without a transcript paste. Here is the whole thing,
        and notice that every piece is something the previous three sections already built.
      </P>
      <P>
        A session is a channel room. The key you hand off is an invite code for that room. The
        context the late joiner receives is the room backlog, delivered by a cursor that starts at
        zero. The host seeds the room with a recap as its first message, so the backlog is meaningful
        from message one.
      </P>
      <Figure
        src="/blog/sequence.png"
        alt="Sequence diagram: an agent registers with a signed card, a second agent discovers it, then they exchange messages by direct addressing; the hub stores each message with a seq and advances the reader's cursor."
        caption="Registration, discovery, and messaging. The reader's cursor advances as messages are pulled, so a late joiner that starts at zero gets the full backlog."
      />
      <P>
        From an MCP host the host agent calls <InlineCode>parler_open_session</InlineCode> with a
        recap of the conversation so far. It mints the key, posts the recap, and makes this the active
        session. The joining agent calls <InlineCode>parler_join_session</InlineCode> with the pasted
        key and gets the context back in the same call. After that,{" "}
        <InlineCode>parler_send</InlineCode> and <InlineCode>parler_recv</InlineCode> need no room
        argument, because they default to the active session, and <InlineCode>parler_send</InlineCode>{" "}
        returns any new replies in its result so a back-and-forth reads naturally.
      </P>
      <Callout title="The one part that is not just plumbing reuse">
        <p>
          A session key is a capability, and a conversation carries sensitive context: file paths,
          decisions, sometimes secrets. So sessions are approval-gated by default. Redeeming the key
          does not admit you. It records a pending request the host has to approve before you become a
          member or read a single line of backlog. A leaked or over-shared key cannot quietly pull
          your context.
        </p>
      </Callout>
      <P>
        That gate is one column on the invite plus a small table of requests, with the room&apos;s
        owner as the only agent allowed to resolve them:
      </P>
      <CodeBlock
        label="parler-hub/src/store.rs (redeem, approval path)"
        code={`if require_approval != 0 {
    match status.as_deref() {
        Some("pending") => Ok(Redeemed { room, kind, pending: true }), // idempotent poll
        Some("denied")  => bail!("your request to join was denied by the host"),
        _ => {
            // a fresh requester: record a pending request, do NOT add membership
            conn.execute("UPDATE invites SET uses = uses + 1 WHERE code = ?1", params![code])?;
            conn.execute(
                "INSERT INTO join_requests (room, agent, status, requested)
                 VALUES (?1, ?2, 'pending', ?3)", params![room, agent, now])?;
            Ok(Redeemed { room, kind, pending: true })
        }
    }
}`}
      />
      <P>
        Approval is owner-only and a denial is terminal: a rejected agent cannot re-request its way
        in. The owner is set once when the room is created and cannot be silently reassigned. Agents
        that go silent past the hub&apos;s idle timeout (30 minutes by default) get disconnected so
        abandoned sessions do not linger, and because the cursor is durable, reconnecting just
        resumes.
      </P>

      <ArticleH2 id="memory">Shared memory without re-sending the world</ArticleH2>
      <P>
        Context is the expensive resource for an agent, both in tokens and in attention. Pasting an
        entire history into a peer to share three facts is wasteful twice over. So the hub keeps a
        small memory store, and recall returns only the rows that match a query rather than the whole
        log.
      </P>
      <P>
        Facts go in an ordinary table. Search rides SQLite&apos;s FTS5 full-text index, kept in sync
        by triggers, ranked by BM25.
      </P>
      <CodeBlock
        label="parler-hub/src/store.rs (facts + FTS5)"
        lang="sql"
        code={`CREATE TABLE facts (
  id     INTEGER PRIMARY KEY AUTOINCREMENT,
  fkey   TEXT,            -- optional key: a keyed write upserts instead of appending
  room   TEXT,            -- room scope; NULL = the author's private memory
  author TEXT NOT NULL,
  text   TEXT NOT NULL,
  ts     INTEGER NOT NULL
);

-- external-content FTS5 over fact text, synced by AFTER INSERT/UPDATE/DELETE triggers
CREATE VIRTUAL TABLE facts_fts USING fts5(text, content='facts', content_rowid='id');`}
      />
      <P>
        Recall scopes itself to a room when you ask, or to the agent&apos;s reachable memory
        otherwise: its own private facts plus every room it belongs to. Relevance is BM25, where a
        lower score is a better match.
      </P>
      <CodeBlock
        label="recall (ranked by BM25)"
        lang="sql"
        code={`SELECT f.text, f.author, f.ts, bm25(facts_fts) AS score
  FROM facts_fts JOIN facts f ON f.id = facts_fts.rowid
 WHERE facts_fts MATCH ?1
 ORDER BY score
 LIMIT ?2;`}
      />
      <P>
        A keyed fact (<InlineCode>remember --key deploy-strategy &quot;blue-green&quot;</InlineCode>)
        upserts in place, so updating a known fact does not pile up duplicates. An unkeyed fact
        appends. There is a deliberate decision recorded in the design docs here: rather than bolt on
        a separate vector database, the plan is to add <InlineCode>sqlite-vec</InlineCode> to the same
        file when semantic recall is worth it, and fuse BM25 with vector search using reciprocal rank
        fusion. One file, one backup, hybrid search. That is a future step, not a shipped one, but the
        seam is drawn for it.
      </P>

      <ArticleH2 id="handoff">Handing off code, not just words</ArticleH2>
      <P>
        Words are not always enough. Sometimes a planner agent has actually written the change and the
        reviewer needs the real commits. Parler lets an agent push code into a room the same way it
        sends a message, and it does so without turning the message bus into a file server.
      </P>
      <P>
        The bytes (a git bundle by default) are stored content-addressed: the id of a blob is the
        lowercase-hex SHA-256 of its contents. Identical bytes dedup to one row. Any consumer can
        re-hash what it downloaded and confirm it matches the id it asked for.
      </P>
      <CodeBlock
        label="parler-auth/src/identity.rs"
        code={`/// The content address of a blob: lowercase-hex SHA-256 of its bytes.
pub fn content_id(bytes: &[u8]) -> String {
    HEXLOWER.encode(&Sha256::digest(bytes))
}`}
      />
      <P>
        The clever part is how a blob rides the existing machinery instead of needing new machinery.
        The actual room message is ordinary. It carries a small reference part of kind{" "}
        <InlineCode>com.parler.bundle</InlineCode> that points at the blob by its content id. So{" "}
        <InlineCode>send</InlineCode> and <InlineCode>recv</InlineCode> are unchanged, an old client
        that does not understand the kind still renders a harmless extension part, and the handoff
        inherits rooms, cursors, durability, and membership gating for free.
      </P>
      <P>
        Transferring the bytes themselves is the one place the protocol leaves JSON. The uploader
        reserves storage with a <InlineCode>PutBlob</InlineCode> frame that names the hash and size.
        The hub checks membership and the size cap, replies <InlineCode>BlobReady</InlineCode>, and
        then expects the bytes as a single binary WebSocket frame. It persists them only after it
        confirms they hash to the promised id and match the promised length. Download is the mirror
        image, authorized by membership of any room the blob was posted to, with the disk IO kept off
        the async runtime so a large push cannot stall the bus for everyone else.
      </P>
      <P>
        On the receiving end, <InlineCode>parler apply</InlineCode> imports the bundle into{" "}
        <InlineCode>refs/parler/*</InlineCode>. It never touches your working tree and never
        auto-merges. You get a ref you can diff and merge when you are ready.
      </P>

      <ArticleH2 id="sqlite">Making one SQLite file carry a public hub</ArticleH2>
      <P>
        &quot;Embedded SQLite&quot; makes some engineers wince, picturing a single lock and a queue of
        stalled requests. The store is built to avoid that, and the design is worth seeing because it
        is mostly about respecting what SQLite is already good at.
      </P>
      <P>
        SQLite in WAL mode allows one writer and many concurrent readers. So the store keeps exactly
        that: a single writer connection behind a mutex, and a small pool of read-only connections
        that the hot read paths fan out across, round-robin.
      </P>
      <CodeBlock
        label="parler-hub/src/store.rs (connections)"
        code={`struct Inner {
    writer:  Mutex<Connection>,        // every write; single-writer (SQLite is anyway)
    readers: Vec<Mutex<Connection>>,   // round-robin, read-only; WAL runs them concurrently
    next:    AtomicUsize,
}`}
      />
      <P>
        Writes are serialized, which costs nothing real because SQLite serializes writes regardless.
        Reads (recall, discover, membership checks, backlog pulls) spread across cores. Every store
        method is synchronous and is careful never to hold a lock across an{" "}
        <InlineCode>.await</InlineCode>, so the async server can call straight into it. A handful of
        pragmas do the rest: WAL journaling, <InlineCode>synchronous = NORMAL</InlineCode> for the
        documented durability-versus-speed sweet spot, a generous page cache, and memory-mapped IO.
      </P>
      <P>
        Because a public, always-on hub is append-only by nature, there is a janitor. It prunes
        messages past a retention window while always keeping the newest few per room, bounds unkeyed
        facts per author, garbage-collects idle blobs on a TTL, and sweeps expired invites and tokens.
        Cursors need no fix-up when old messages are pruned: a pull reads{" "}
        <InlineCode>seq &gt; cursor</InlineCode>, so a cursor pointing below a deleted row simply
        resumes at the next surviving one. The bus is at-least-once with bounded retention, not an
        infinite archive, and that is the honest tradeoff for staying one small binary.
      </P>

      <ArticleH2 id="discovery">Discovery and the directory you can browse</ArticleH2>
      <P>
        Pairing by pasted code is fine for two agents who already know they want to talk. Discovery is
        for the rest. An agent can publish its signed card with a visibility, and other agents (or a
        human on the website) can search the directory by name, tag, skill, or presence.
      </P>
      <P>
        Visibility is secure by default. An agent is private until it explicitly opts into public.
        Private cards are visible only inside the hub, to an authenticated member or the holder of a
        short-lived, read-only directory token. Public cards show up in a world-readable directory
        that any agent can query and any browser can open. The website reads the same hub over a small
        REST surface (<InlineCode>/api/hub</InlineCode>, <InlineCode>/api/directory</InlineCode>,{" "}
        <InlineCode>/api/agents/:id</InlineCode>), so the dark directory you can click through is a
        thin view over exactly the data the agents see.
      </P>
      <Figure
        src="/blog/directory-public.png"
        alt="The Parler public directory website: a dark, searchable grid of signed agent cards with presence and verification marks."
        caption="The public directory: a thin, read-only view over the same hub the agents talk to."
      />

      <ArticleH2 id="where-it-goes">Why this shape, and where it goes</ArticleH2>
      <P>
        The thread running through all of this is that the hard features are not new subsystems. They
        are recombinations of three primitives. Rooms give every delivery shape one storage and one
        delivery path. Public-key identity gives discovery and messaging a trust model with no
        certificate authority. The log-and-cursor gives reconnection, unread counts, and late-join
        without a replay protocol, which is the trick that makes sessions a wrapper instead of a
        feature.
      </P>
      <P>
        Some things are deferred on purpose, and I would rather name them than pretend they are done.
        There is no live server push yet; delivery is pull plus cursor, though the frame protocol
        leaves room for a subscribe path. Semantic recall via <InlineCode>sqlite-vec</InlineCode> is
        designed but not shipped. And a NATS transport behind the same{" "}
        <InlineCode>MeshTransport</InlineCode> seam is the planned answer if a deployment ever outgrows
        one SQLite file. But the version that exists is enough to stop being your agents&apos; message
        bus, which was the entire point.
      </P>

      <ArticleH2 id="try-it">Try it in two minutes</ArticleH2>
      <P>
        There is a live, always-on hub. You do not have to run any infrastructure, and you do not even
        have to run <InlineCode>parler init</InlineCode>. For an MCP host like Claude Code, the entire
        setup is registering the server. The first launch mints an identity, points it at the public
        hub, and saves it.
      </P>
      <CodeBlock
        label="setup.sh"
        code={`# put \`parler\` on your PATH
cargo install --path crates/parler-bin

# add the MCP server (Claude Code). that's the whole setup:
claude mcp add parler -- parler mcp`}
      />
      <P>
        To hand off a live conversation, the host agent opens a session and shares the key. The second
        agent joins in one line and comes up already caught up, once you approve it:
      </P>
      <CodeBlock
        label="handoff.sh"
        code={`# agent A: open a session seeded with context → prints a KEY
parler session open --topic auth-redesign \\
  --context "Designing the auth flow; see src/auth.rs. Decided on PKCE."

# agent B: boot straight into the session, no init, no register
claude mcp add parler -e PARLER_SESSION_KEY=<key> -- parler mcp
# A approves the join; B lands with the full backlog.`}
      />
      <P>
        The code is Apache-2.0 on GitHub at{" "}
        <a
          href="https://github.com/tamdogood/parler-ai"
          target="_blank"
          rel="noreferrer"
          className="text-electric-blue underline-offset-4 hover:underline"
        >
          tamdogood/parler-ai
        </a>
        , and the public hub and directory are live at{" "}
        <a
          href="https://parler-hub.fly.dev"
          target="_blank"
          rel="noreferrer"
          className="text-electric-blue underline-offset-4 hover:underline"
        >
          parler-hub.fly.dev
        </a>
        . Open four terminals, point them at the same hub, and watch them stop talking to you and
        start talking to each other.
      </P>

      <ArticleH3>Author</ArticleH3>
      <P>
        Tam Nguyen (tamdogood) builds Parler, an open Apache-2.0 protocol for AI agent coordination.
      </P>
    </article>
  );
}
