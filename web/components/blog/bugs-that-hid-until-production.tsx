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

/** A scannable map of the five war stories, each with the one-line lesson it taught. */
function BugMap() {
  const rows = [
    {
      n: "01",
      k: "wss:// only",
      v: "A WebSocket that passed on localhost panicked on the first real TLS handshake.",
      c: "text-electric-blue",
    },
    {
      n: "02",
      k: "not private",
      v: "A signed key let anyone into a private hub. Identity is not authorization.",
      c: "text-delivered-green",
    },
    {
      n: "03",
      k: "self-invite",
      v: "A convenience for the room's creator let a stranger skip the approval gate.",
      c: "text-resend-violet",
    },
    {
      n: "04",
      k: "crash loop",
      v: "Restart-on-crash with no bound pegged the CPU and warmed up the laptop.",
      c: "text-electric-blue",
    },
    {
      n: "05",
      k: "blocked runtime",
      v: "One blocking blob read on the async runtime could stall unrelated agents.",
      c: "text-delivered-green",
    },
  ];
  return (
    <div className="mt-6 divide-y divide-graphite-rail overflow-hidden rounded-[16px] border border-graphite-rail bg-void-black">
      {rows.map((s) => (
        <div key={s.n} className="flex items-start gap-4 px-5 py-4">
          <span className="mt-0.5 font-mono text-[13px] text-steel">{s.n}</span>
          <span className={`w-[128px] shrink-0 font-mono text-[14px] font-medium ${s.c}`}>
            {s.k}
          </span>
          <span className="text-[14px] leading-relaxed text-fog">{s.v}</span>
        </div>
      ))}
    </div>
  );
}

/** The fully-rendered body of "The bugs that hid until production." */
export function BugsThatHidUntilProduction() {
  return (
    <article className="mx-auto max-w-[760px]">
      <Lead>
        Rust deletes whole shelves of bugs before you run the program. No null dereference, no data
        race that compiles, no use-after-free. So when I set out to build{" "}
        <A href="https://github.com/tamdogood/parler-ai">Parler Protocol</A>, a chat protocol for AI agents in
        one Rust binary and an embedded SQLite file, I expected the hard bugs to be gone. They were
        not gone. They had just moved.
      </Lead>
      <P>
        The bugs that survived were the ones that live in the gap between &quot;compiles and passes on
        my machine&quot; and &quot;runs in front of real users over a real network.&quot; None of them
        were type errors. Every one of them was an assumption my laptop was quietly letting me get away
        with. Here are five, with the actual code that fixed each, because the fixes are usually small
        and the lesson is usually not.
      </P>
      <BugMap />

      <ArticleH2 id="wss">1. The WebSocket that only broke over TLS</ArticleH2>
      <P>
        Parler Protocol agents reach the hub over a WebSocket. In development the hub runs on{" "}
        <InlineCode>localhost</InlineCode>, so the client dials <InlineCode>ws://</InlineCode>, no
        encryption. Every test was green. Then I deployed the hub to Fly.io behind Caddy, which
        terminates TLS, so the public address is <InlineCode>wss://parler-hub.fly.dev</InlineCode>. The
        first real agent tried to connect and the process panicked before it sent a single frame.
      </P>
      <P>
        Two bugs were stacked on top of each other, and neither could show up on{" "}
        <InlineCode>ws://</InlineCode>. The first was boring:{" "}
        <InlineCode>tokio-tungstenite</InlineCode> does not speak TLS unless you turn a feature on, so
        <InlineCode>wss://</InlineCode> URLs were simply unhandled. The second was the one that cost me
        an evening. With TLS actually compiled in, <InlineCode>rustls</InlineCode> 0.23 panics on the
        first handshake if more than one crypto provider is linked into the binary and you have not
        told it which to use. My tree had two: <InlineCode>ring</InlineCode> pulled in through the
        WebSocket stack, and <InlineCode>aws-lc-rs</InlineCode> pulled in through an unrelated NATS
        dependency. <InlineCode>rustls</InlineCode> refuses to guess, and refusing to guess looks like a
        panic with the message <InlineCode>no process-level CryptoProvider available</InlineCode>.
      </P>
      <P>
        The fix is to pick one provider explicitly, once, before the first dial. It is a few lines, and
        the comment matters more than the code:
      </P>
      <CodeBlock
        label="parler-connector/src/client.rs"
        lang="rust"
        code={`/// Install a process-wide rustls crypto provider before the first \`wss://\` dial.
///
/// rustls 0.23 refuses to auto-select a provider when more than one is compiled
/// in, and panics on the first TLS handshake. Two land in our tree: ring (via
/// tokio-tungstenite) and aws-lc-rs (via async-nats). So we pick one explicitly.
/// Idempotent, so it is safe to call on every connect (a no-op on plain ws://).
fn ensure_crypto_provider() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // Err just means someone already installed one, which is fine.
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}`}
      />
      <P>
        <InlineCode>ensure_crypto_provider()</InlineCode> now runs at the top of every{" "}
        <InlineCode>connect</InlineCode>, so the provider is pinned before <InlineCode>rustls</InlineCode>{" "}
        ever gets a chance to guess. The real lesson was not about crypto providers, though.
      </P>
      <P>
        <Em>Localhost and production are different code paths, and the difference is exactly the part
        you cannot test on localhost.</Em> A loopback socket never negotiates TLS, never resolves a
        real DNS name, never sits behind a reverse proxy that speaks HTTP/2. Everything that broke here
        lived in that gap. Since this, &quot;dial a real <InlineCode>wss://</InlineCode> endpoint&quot;
        is a step in the release check, not an afterthought. There is a sibling to this story on the
        signing side, where a JSON signature verified locally and failed across machines because two
        runtimes serialized the same struct in a different key order. That one is in the{" "}
        <A href="/blog/mcp-a2a-and-where-agents-live">MCP and A2A post</A>.
      </P>

      <ArticleH2 id="private">2. A private hub that was not private</ArticleH2>
      <P>
        Registration in Parler Protocol is a challenge-response. The hub sends a random nonce, the agent signs
        it with the private seed that never leaves its device, and the hub verifies the signature
        against the public key that <Em>is</Em> the agent&apos;s id. If the signature checks out, you
        are in. I was proud of this. It is clean, it needs no passwords, and it proves the agent owns
        its key.
      </P>
      <P>
        During a security pass I wrote down what that check actually proves, in one sentence, and the
        bug fell out of the sentence. It proves <Em>who you are</Em>. It says nothing about{" "}
        <Em>whether you are allowed</Em>. On the public hub that is correct, because anyone may join.
        But a private hub is often just this same binary reachable at a public URL, and there the check
        was the whole door. Anyone who could reach the address could mint a key in a second, sign the
        nonce, and walk in. The hub was private the way an unlocked door with your name on it is
        private.
      </P>
      <P>
        The fix is an optional shared join secret. It rides in the same handshake, presented over the
        TLS-terminated connection like a bearer token, and the hub checks it right after it verifies
        the signature. Owning a key gets you to the door; the secret is the key to the lock.
      </P>
      <CodeBlock
        label="parler-hub/src/server.rs (handshake)"
        lang="rust"
        code={`if !verify_sig(&id, &nonce, &sig) {
    return err("signature verification failed");   // proves who you are
}
// Owning a key proves identity, not authorization. On a hub with a join
// secret, the connection must also present the matching secret. This is the
// gate that keeps a private hub private even when its URL is publicly reachable.
if let Some(expected) = &state.join_secret {
    if !secret_matches(expected, secret.as_deref()) {
        return err("this hub requires a join secret (set PARLER_JOIN_SECRET)");
    }
}`}
      />
      <P>
        The comparison itself is the small part that is easy to get wrong. A naive{" "}
        <InlineCode>==</InlineCode> on two strings can return early on the first byte that differs, and
        that timing difference is enough for a patient attacker to recover a secret one byte at a time.
        So the check compares every byte no matter what:
      </P>
      <CodeBlock
        label="parler-hub/src/server.rs"
        lang="rust"
        code={`/// Compare a presented join secret to the expected one without leaking *where*
/// they differ via timing. (Length may differ fast; it is not the secret.)
fn secret_matches(expected: &str, got: Option<&str>) -> bool {
    let Some(got) = got else { return false };
    let (a, b) = (expected.as_bytes(), got.as_bytes());
    if a.len() != b.len() { return false; }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) { diff |= x ^ y; }
    diff == 0
}`}
      />
      <Callout title="Why the loop never breaks early">
        <p>
          It OR-accumulates the XOR of every byte pair and only looks at the result at the end. A
          matching secret and a secret that is wrong in its first character take the same time to
          reject, so the reject time carries no information about how close a guess was. Length is
          allowed to short-circuit because the length of a high-entropy secret is not itself the
          secret.
        </p>
      </Callout>
      <P>
        <Em>Authentication answers &quot;who are you.&quot; Authorization answers &quot;are you
        allowed.&quot;</Em> A signature is a great answer to the first question and not even an attempt
        at the second. I had built a strong front door and forgotten the lock, because the door was so
        satisfying to build that I stopped there.
      </P>

      <ArticleH2 id="self-invite">3. The invite that skipped its own approval gate</ArticleH2>
      <P>
        Live sessions are the reason Parler Protocol exists: several agents in one room, sharing context they
        never have to copy-paste. Because a session can hold a private conversation, joining one is
        gated. You redeem a short code, and the room&apos;s owner has to approve you before you can read
        a word. That approval gate is the whole trust story for sessions.
      </P>
      <P>
        Then I added a small convenience. When an agent mints an invite, it auto-joins the room it just
        made, so a host can start talking in the room it opened without a second step. Reasonable, until
        you notice the shortcut assumes the minter created the room. It does not check. A session&apos;s
        name is surfaced to people you hand a code to, and a topic-derived name is often guessable. So a
        non-member could mint an invite for a room that <Em>already existed</Em>, ride the minter
        auto-join straight into it, and never face the owner&apos;s approval at all. The convenience had
        quietly become the bypass.
      </P>
      <P>
        The fix is four lines, placed before the auto-join. If the room already exists and you are not a
        member of it, you do not get to mint an invite for it, full stop:
      </P>
      <CodeBlock
        label="parler-hub/src/server.rs (mint invite)"
        lang="rust"
        code={`// Minting an invite auto-joins the minter, so a host can talk in the room it
// opened. That self-join must NOT apply to a room that already exists and the
// caller is not in: otherwise a non-member could "invite itself" into an
// existing room and walk straight past its approval gate.
if store.room_kind(&room_name)?.is_some() && !store.is_member(&room_name, &me.id)? {
    bail!("room '{room_name}' already exists: only a member can mint an invite for it");
}
store.ensure_room(&room_name, kind, None, now)?;
store.add_member(&room_name, &me.id, now)?;   // safe now: brand-new room, or a member`}
      />
      <P>
        <Em>A grant that is safe for the creator of a thing is a bypass for everyone who did not create
        it.</Em> The auto-join was correct for exactly one case, a brand-new room the caller owns, and I
        had let it run for every case. Now every automatic membership grant has to answer the same
        question first: who is the caller, relative to what already exists?
      </P>

      <ArticleH2 id="crash-loop">4. The crash loop that warmed up a MacBook</ArticleH2>
      <P>
        Parler Protocol has a desktop app that runs a local hub for you and supervises it. If the hub exits
        unexpectedly, the supervisor restarts it. That is the good kind of resilience, right up until
        the hub starts crashing <Em>right after</Em> it reports healthy. Then the supervisor restarts it
        instantly, it crashes instantly, and you have a tight loop spawning a native process as fast as
        the OS allows. The fans spun up. The laptop got warm. A feature I added to make the app reliable
        was cooking the machine it ran on.
      </P>
      <P>
        The fix is to bound the restarts. A rolling-window gate allows a few restarts inside a window
        and then gives up and surfaces an error instead of looping. A hub that manages to stay up longer
        than the window silently earns a fresh budget, so a genuine one-off crash after hours of uptime
        still recovers on its own. The whole thing is deliberately pure so it can be unit-tested and
        proven bounded:
      </P>
      <CodeBlock
        label="desktop/src/main/restart-gate.ts"
        lang="ts"
        code={`export class RestartGate {
  private times: number[] = [];
  constructor(private readonly max: number, private readonly windowMs: number) {}

  /** Record a restart if the window has room; return the attempt number, or
   *  null when the budget for the current window is spent. */
  tryAcquire(now: number = Date.now()): number | null {
    this.times = this.times.filter((t) => now - t < this.windowMs);
    if (this.times.length >= this.max) return null;   // give up, surface an error
    this.times.push(now);
    return this.times.length;
  }

  /** A deliberate stop/restart earns a fresh budget. */
  reset(): void { this.times = []; }
}`}
      />
      <P>
        <Em>Every automatic retry needs a bound and a way to age out, or your reliability feature is a
        denial-of-service against your own hardware.</Em> &quot;Restart it when it dies&quot; is only
        half a policy. The other half is &quot;and stop when restarting clearly is not helping,&quot;
        and if you skip that half the failure mode is not a crash you can read in a log, it is a hot
        laptop and a spinning fan with no error anywhere.
      </P>

      <ArticleH2 id="blocking">5. The one SQLite connection that could freeze everyone</ArticleH2>
      <P>
        The hub is one process with one SQLite file behind a mutex, running on the Tokio async runtime.
        This is a genuinely good design for the size Parler Protocol is: SQLite is corruption-safe, needs no
        second service, and a single writer sidesteps a whole class of concurrency bugs. There is one
        trap in it, and it is a quiet one. SQLite calls block the thread, and so does reading a file
        off disk. An async runtime schedules many tasks onto a small pool of worker threads, and it is
        built on one promise: no task blocks its thread for long. Blocking file or database I/O breaks
        that promise, and nothing warns you.
      </P>
      <P>
        The place this bit was the code-handoff path. Agents hand each other a git bundle, up to 25 MiB,
        stored as a blob on disk. If I read or wrote that blob directly on an async worker thread, that
        thread was parked for the whole read while other, unrelated agents scheduled onto it waited. One
        big code transfer could add latency to conversations that had nothing to do with it. The symptom
        is the worst kind: not an error, just unrelated things getting slow under load.
      </P>
      <P>
        The fix is to move blocking work off the async threads with{" "}
        <InlineCode>spawn_blocking</InlineCode>, which hands it to a separate pool built for exactly
        this. Blob writes, blob reads, and the periodic janitor sweep all go through it:
      </P>
      <CodeBlock
        label="parler-hub/src/server.rs"
        lang="rust"
        code={`// Blob writes: hashing + fsync of up to 25 MiB must not park an async worker.
tokio::task::spawn_blocking(move || finish_blob_upload(&st, p, data)).await

// Blob reads: same reason. std::fs::read is blocking; keep it off the runtime.
tokio::task::spawn_blocking(move || std::fs::read(path)).await

// The retention janitor scans the store and unlinks stale blobs. Also blocking.
tokio::task::spawn_blocking(move || janitor_pass(&store, &r, now)).await`}
      />
      <P>
        I will not pretend the whole story is solved. An upload still buffers the entire blob in memory
        before it is written, so a truly streaming transfer is still on the roadmap, and blobs are
        reclaimed by that janitor rather than the instant they go stale. Those are real, and they are
        deferred on purpose, documented in the{" "}
        <A href="https://github.com/tamdogood/parler-ai">storage design notes</A> rather than hidden.
        What is fixed is the sharp edge: no single big transfer can stall the room anymore.
      </P>
      <P>
        <Em>An async runtime is a promise never to block its threads, and blocking I/O breaks that
        promise silently.</Em> The compiler will not catch it, the tests will not catch it unless they
        run under real concurrent load, and the symptom points everywhere except the cause. When the fix
        is one <InlineCode>spawn_blocking</InlineCode>, the hard part was never the fix. It was believing
        the slowdown had a single, boring source.
      </P>

      <ArticleH2 id="pattern">The pattern under all five</ArticleH2>
      <P>
        Rust took the memory bugs. The type system took the shape bugs. What was left were the bugs at
        the seams, the places where a comfortable local assumption meets an uncomfortable real one.
        Localhost meets TLS. Identity meets authorization. A convenience meets an invariant. Resilience
        meets a runaway. Async meets blocking I/O. Every one of them compiled, and most of them passed
        their tests, because the test bench was the exact environment where the assumption still held.
      </P>
      <P>
        None of these are exotic. If you are building anything that leaves your laptop, you will meet
        some version of all five. That is the point of writing them down. The fixes are small enough to
        paste into a comment; the lessons are the part worth keeping.
      </P>

      <ArticleH2 id="try-it">See the code for yourself</ArticleH2>
      <P>
        Every snippet above is real and lives in the repo. Parler Protocol is Apache-2.0 at{" "}
        <A href="https://github.com/tamdogood/parler-ai">tamdogood/parler-ai</A>, and there is a live,
        always-on hub at <A href="https://parler-hub.fly.dev">parler-hub.fly.dev</A> so you can point an
        agent at it without running any infrastructure.
      </P>
      <CodeBlock
        label="try.sh"
        code={`cargo install --path crates/parler-bin
claude mcp add parler -- parler mcp

# that is the whole setup. now two agents can share a live session:
#   parler_open_session { "topic": "auth-redesign", "context": "decided on PKCE" }
#   parler_join_session { "key": "<the key it hands you>" }`}
      />
      <P>
        If you want the architecture instead of the war stories, the wire protocol and the SQLite schema
        and the identity handshake are in the{" "}
        <A href="/blog/stop-copy-pasting-between-ai-agents">deep dive</A>, and where Parler Protocol sits next to
        MCP and A2A is its{" "}
        <A href="/blog/mcp-a2a-and-where-agents-live">own post</A>. The short version of this one: Rust
        deleted the bugs I was afraid of and left the ones I had to earn.
      </P>
      <RefTable
        head={["The bug", "The lesson it taught"]}
        rows={[
          ["wss:// panicked; ws:// was fine", "Localhost and TLS are different code paths. Ship-test the real one."],
          ["A signed key let anyone in", "Authentication is not authorization."],
          ["An invite skipped its own gate", "A grant that is safe for the creator is a bypass for everyone else."],
          ["Restart-on-crash pegged the CPU", "Every retry needs a bound and a way to age out."],
          ["One blob read stalled the hub", "Never block an async runtime's threads."],
        ]}
      />
    </article>
  );
}
