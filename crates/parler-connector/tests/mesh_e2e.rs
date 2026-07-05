//! End-to-end: a real in-process hub + real WebSocket clients exercising the whole feature —
//! the three delivery patterns, paste-a-code pairing, memory scoping, durable resume, and authz.

use parler_connector::{
    verify_message, BundleMeta, Config, HubClient, JoinOutcome, MeshAgent, MeshTransport, SigStatus,
};
use parler_protocol::{
    BundleRef, ClientFrame, EndpointRef, Part, RoomKind, ServerFrame, StoredMessage, Target,
};
use std::sync::Arc;
use std::time::Duration;

/// Start an in-memory hub on an ephemeral port; return its ws:// URL.
async fn start_hub() -> String {
    let store = parler_hub::Store::open(None).unwrap();
    let state = Arc::new(parler_hub::HubState::new(
        store,
        "parler://test".into(),
        "Test Hub".into(),
        parler_hub::HubMode::Private,
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = parler_hub::serve(listener, state).await;
    });
    format!("ws://{addr}")
}

/// Start an in-memory hub with a specific authenticated-idle timeout (`None` disables it).
async fn start_hub_with_idle(idle: Option<Duration>) -> String {
    let store = parler_hub::Store::open(None).unwrap();
    let mut state = parler_hub::HubState::new(
        store,
        "parler://test".into(),
        "Test Hub".into(),
        parler_hub::HubMode::Private,
    );
    state.idle_timeout = idle;
    let state = Arc::new(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = parler_hub::serve(listener, state).await;
    });
    format!("ws://{addr}")
}

/// A transport decorator that simulates a **half-open** socket on demand: while `armed` is set, the
/// *next* `request` hangs forever (as a real half-open read would), then disarms. `MeshAgent`'s
/// heartbeat times out on the hang and forces a reconnect — which rebuilds a fresh real client past
/// this decorator — so the caller heals transparently. Start disarmed so setup (join) runs normally;
/// arm it right before the long-poll that must survive the drop.
struct FlakyTransport {
    inner: Box<dyn MeshTransport>,
    armed: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl FlakyTransport {
    fn new(inner: Box<dyn MeshTransport>) -> (Self, std::sync::Arc<std::sync::atomic::AtomicBool>) {
        let armed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        (FlakyTransport { inner, armed: armed.clone() }, armed)
    }
}

#[async_trait::async_trait]
impl MeshTransport for FlakyTransport {
    async fn request(&mut self, frame: ClientFrame) -> anyhow::Result<ServerFrame> {
        if self.armed.swap(false, std::sync::atomic::Ordering::SeqCst) {
            // A half-open read never completes; block past the heartbeat timeout so the heartbeat
            // gives up on this connection and reconnects. (This future is dropped when the heartbeat's
            // `timeout` elapses and `reconnect` replaces the whole transport.)
            std::future::pending::<()>().await;
        }
        self.inner.request(frame).await
    }

    async fn subscribe(&mut self) -> anyhow::Result<bool> {
        self.inner.subscribe().await
    }

    async fn next_delivery(&mut self, max_wait: Duration) -> anyhow::Result<Option<StoredMessage>> {
        self.inner.next_delivery(max_wait).await
    }
}

fn cfg(hub: &str, name: &str, role: Option<&str>) -> Config {
    Config {
        hub_url: hub.to_string(),
        identity: parler_auth::new_identity().unwrap(),
        name: name.to_string(),
        role: role.map(String::from),
    }
}

async fn agent(hub: &str, name: &str, role: Option<&str>) -> MeshAgent {
    MeshAgent::connect(&cfg(hub, name, role)).await.unwrap()
}

fn texts(msgs: &[StoredMessage]) -> Vec<String> {
    msgs.iter()
        .map(|m| {
            m.parts
                .iter()
                .filter_map(|p| match p {
                    Part::Text(t) => Some(t.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect()
}

#[tokio::test]
async fn one_to_one_dm_pairing_round_trips() {
    let hub = start_hub().await;
    let mut alice = agent(&hub, "alice", None).await;
    let mut bob = agent(&hub, "bob", None).await;

    // alice mints a DM invite; bob pastes the code to join.
    let inv = alice.invite(RoomKind::Dm, None, None, None).await.unwrap();
    let (broom, kind) = bob.join(&inv.code).await.unwrap();
    assert_eq!(kind, RoomKind::Dm);
    assert_eq!(broom, inv.room);

    // Two-way DM addressed by peer id (the hub resolves the shared DM room).
    alice.send_text(Target::Dm { agent: bob.id.clone() }, "hey bob").await.unwrap();
    let (m, _) = bob.pull(&broom, None, None).await.unwrap();
    assert_eq!(texts(&m), vec!["hey bob"]);

    bob.send_text(Target::Dm { agent: alice.id.clone() }, "hi alice").await.unwrap();
    // alice's cursor was still at 0, so her first pull returns the whole thread (her own send
    // included) — the durable inbox returns everything past the cursor; filtering own messages is
    // the consumer's choice.
    let (m2, _) = alice.pull(&inv.room, None, None).await.unwrap();
    assert_eq!(texts(&m2), vec!["hey bob", "hi alice"]);
}

#[tokio::test]
async fn one_to_many_channel_fans_out() {
    let hub = start_hub().await;
    let mut alice = agent(&hub, "alice", None).await;
    let mut bob = agent(&hub, "bob", None).await;
    let mut carol = agent(&hub, "carol", None).await;

    let inv = alice.invite(RoomKind::Channel, Some("team".into()), None, None).await.unwrap();
    bob.join(&inv.code).await.unwrap();
    carol.join(&inv.code).await.unwrap(); // same code, multi-use channel

    alice.send_text(Target::Room { room: inv.room.clone() }, "standup at 10").await.unwrap();

    let (bm, _) = bob.pull(&inv.room, None, None).await.unwrap();
    let (cm, _) = carol.pull(&inv.room, None, None).await.unwrap();
    assert_eq!(texts(&bm), vec!["standup at 10"]);
    assert_eq!(texts(&cm), vec!["standup at 10"]);
}

#[tokio::test]
async fn many_to_one_service_collects() {
    let hub = start_hub().await;
    let mut manager = agent(&hub, "manager", Some("reviewer")).await;
    let mut alice = agent(&hub, "alice", None).await;
    let mut bob = agent(&hub, "bob", None).await;

    let room = manager.serve("review").await.unwrap();
    alice.send_text(Target::Service { service: "review".into() }, "review PR #1").await.unwrap();
    bob.send_text(Target::Service { service: "review".into() }, "review PR #2").await.unwrap();

    let (msgs, _) = manager.pull(&room, None, None).await.unwrap();
    let t = texts(&msgs);
    assert_eq!(t.len(), 2);
    assert!(t.contains(&"review PR #1".to_string()));
    assert!(t.contains(&"review PR #2".to_string()));
}

#[tokio::test]
async fn memory_recall_respects_scope() {
    let hub = start_hub().await;
    let mut alice = agent(&hub, "alice", Some("planner")).await;
    let mut bob = agent(&hub, "bob", None).await;
    let inv = alice.invite(RoomKind::Channel, Some("team".into()), None, None).await.unwrap();
    bob.join(&inv.code).await.unwrap();

    // A room-scoped fact is recallable by any member.
    alice.remember("the deploy strategy is blue-green", None, Some(inv.room.clone()), None, None).await.unwrap();
    let hits = bob.recall("deploy", Some(inv.room.clone()), None, None).await.unwrap();
    assert_eq!(hits.len(), 1);
    assert!(hits[0].text.contains("blue-green"));

    // A private fact is only recallable by its author.
    alice.remember("my api token is xyz", Some("token".into()), None, None, None).await.unwrap();
    assert_eq!(bob.recall("token", None, None, None).await.unwrap().len(), 0);
    assert_eq!(alice.recall("token", None, None, None).await.unwrap().len(), 1);
}

#[tokio::test]
async fn reconnect_resumes_from_durable_cursor() {
    let hub = start_hub().await;
    let mut alice = agent(&hub, "alice", None).await;
    let bob_cfg = cfg(&hub, "bob", None); // a stable identity we reconnect with

    let inv = alice.invite(RoomKind::Channel, Some("team".into()), None, None).await.unwrap();
    let room = inv.room.clone();

    {
        let mut bob = MeshAgent::connect(&bob_cfg).await.unwrap();
        bob.join(&inv.code).await.unwrap();
        alice.send_text(Target::Room { room: room.clone() }, "first").await.unwrap();
        let (m, _) = bob.pull(&room, None, None).await.unwrap();
        assert_eq!(texts(&m), vec!["first"]);
        // Deferred-ack model (#85): a pull no longer commits the cursor immediately — it's acked on
        // the NEXT pull, so a batch whose reply is lost is re-read rather than skipped. Bob's second
        // pull (empty) carries that ack, durably committing the cursor past "first".
        let (empty, _) = bob.pull(&room, None, None).await.unwrap();
        assert!(empty.is_empty());
    } // bob disconnects, having acked "first"

    alice.send_text(Target::Room { room: room.clone() }, "second").await.unwrap();

    // A fresh connection for the same identity resumes from the durable (acked) cursor.
    let mut bob2 = MeshAgent::connect(&bob_cfg).await.unwrap();
    let (m2, _) = bob2.pull(&room, None, None).await.unwrap();
    assert_eq!(texts(&m2), vec!["second"]);
}

#[tokio::test]
async fn code_handoff_push_recv_fetch_round_trips() {
    let hub = start_hub().await;
    let mut alice = agent(&hub, "alice", None).await;
    let mut bob = agent(&hub, "bob", None).await;
    let inv = alice.invite(RoomKind::Channel, Some("dev".into()), None, None).await.unwrap();
    bob.join(&inv.code).await.unwrap();

    // The hub treats the bundle as opaque bytes, so any payload exercises the transport.
    let bundle = b"PARLER-FAKE-GIT-BUNDLE\x00\x01\x02 some commits here".to_vec();
    let meta = BundleMeta {
        vcs: "git".into(),
        tip: Some("abc123def".into()),
        base: None,
        summary: Some("feat: add the thing".into()),
        media_type: Some("application/x-git-bundle".into()),
    };
    let receipt = alice
        .push(Target::Room { room: inv.room.clone() }, &bundle, meta, Some("here's the patch".into()))
        .await
        .unwrap();

    // Bob sees the handoff as an ordinary message carrying a bundle reference (+ the note).
    let (msgs, _) = bob.pull(&inv.room, None, None).await.unwrap();
    assert_eq!(texts(&msgs), vec!["here's the patch"]);
    let bref = msgs
        .iter()
        .flat_map(|m| &m.parts)
        .find_map(BundleRef::from_part)
        .expect("a com.parler.bundle part");
    assert_eq!(bref.blob, receipt.blob_id);
    assert_eq!(bref.summary.as_deref(), Some("feat: add the thing"));
    assert_eq!(bref.size, bundle.len() as u64);

    // Bob fetches the bytes by content id and they match exactly.
    let got = bob.fetch_blob(&bref.blob).await.unwrap();
    assert_eq!(got, bundle);

    // A non-member of the room cannot fetch the blob.
    let mut eve = agent(&hub, "eve", None).await;
    assert!(eve.fetch_blob(&bref.blob).await.is_err());
}

// ---- real-time push delivery ----

#[tokio::test]
async fn push_delivery_is_sub_second() {
    // A subscribed member is pushed a peer's message the instant it's sent — no poll — while the
    // durable cursor stays the source of truth (push never advances it).
    let hub = start_hub().await;
    let mut alice = agent(&hub, "alice", None).await;
    let mut bob = agent(&hub, "bob", None).await;
    let inv = alice.invite(RoomKind::Channel, Some("live".into()), None, None).await.unwrap();
    bob.join(&inv.code).await.unwrap();

    // Bob opts into push; the hub acks (it supports it).
    assert!(bob.subscribe().await.unwrap(), "hub should support push");
    // Nothing sent yet → a short wait yields nothing (no false wake).
    assert!(bob.next_delivery(Duration::from_millis(100)).await.unwrap().is_none());

    // Alice sends; bob is pushed it well under a second.
    alice.send_text(Target::Room { room: inv.room.clone() }, "live ping").await.unwrap();
    let got = bob
        .next_delivery(Duration::from_secs(2))
        .await
        .unwrap()
        .expect("a pushed delivery");
    assert_eq!(texts(std::slice::from_ref(&got)), vec!["live ping"]);
    assert_eq!(got.room, inv.room);

    // The author is not pushed its own message: alice subscribes, sends, and gets nothing back…
    assert!(alice.subscribe().await.unwrap());
    alice.send_text(Target::Room { room: inv.room.clone() }, "from alice").await.unwrap();
    assert!(
        alice.next_delivery(Duration::from_millis(300)).await.unwrap().is_none(),
        "an author must not be pushed its own message"
    );
    // …but bob (a peer) is.
    let got2 = bob
        .next_delivery(Duration::from_secs(2))
        .await
        .unwrap()
        .expect("bob is pushed the peer message");
    assert_eq!(texts(std::slice::from_ref(&got2)), vec!["from alice"]);

    // Push did NOT advance bob's durable cursor — a pull still returns the whole backlog, proving
    // push is a latency layer over (not a replacement for) the cursor.
    let (pulled, _) = bob.pull(&inv.room, None, None).await.unwrap();
    assert_eq!(texts(&pulled), vec!["live ping", "from alice"]);
}

#[tokio::test]
async fn unsubscribed_agent_is_never_pushed() {
    // Push is opt-in: an agent that didn't `subscribe` is never sent a Delivery, and `next_delivery`
    // returns immediately (it stays a pure puller — the backward-compatible path).
    let hub = start_hub().await;
    let mut alice = agent(&hub, "alice", None).await;
    let mut bob = agent(&hub, "bob", None).await;
    let inv = alice.invite(RoomKind::Channel, Some("quiet".into()), None, None).await.unwrap();
    bob.join(&inv.code).await.unwrap();

    alice.send_text(Target::Room { room: inv.room.clone() }, "no push for you").await.unwrap();
    // Bob never subscribed → no wait, no delivery; the message is still there to pull.
    assert!(bob.next_delivery(Duration::from_millis(200)).await.unwrap().is_none());
    let (m, _) = bob.pull(&inv.room, None, None).await.unwrap();
    assert_eq!(texts(&m), vec!["no push for you"]);
}

#[tokio::test]
async fn non_member_cannot_read_a_room() {
    let hub = start_hub().await;
    let mut alice = agent(&hub, "alice", None).await;
    let mut eve = agent(&hub, "eve", None).await;

    let inv = alice.invite(RoomKind::Channel, Some("secret".into()), None, None).await.unwrap();
    alice.send_text(Target::Room { room: inv.room.clone() }, "classified").await.unwrap();

    // eve never redeemed the invite → not a member → reads are refused.
    assert!(eve.pull(&inv.room, None, None).await.is_err());
}

// ---- live multi-agent sessions (the publish-key / join-with-context flow) ----

#[tokio::test]
async fn live_session_handoff_shares_context_with_many() {
    // The session flow the MCP `parler_open_session` / `parler_join_session` tools compose:
    // a multi-use channel invite (the "key") + a seeded context message + a backlog pull on join.
    let hub = start_hub().await;
    let mut alice = agent(&hub, "alice", Some("planner")).await;

    // Alice opens a session: mint the key, then seed it with the conversation context.
    let inv = alice.invite(RoomKind::Channel, Some("design".into()), None, None).await.unwrap();
    alice
        .send_text(
            Target::Room { room: inv.room.clone() },
            "📋 session context: we are designing the auth flow; see src/auth.rs",
        )
        .await
        .unwrap();

    // Bob joins with the key and pulls the backlog → he is caught up with the context.
    let mut bob = agent(&hub, "bob", None).await;
    let (broom, kind) = bob.join(&inv.code).await.unwrap();
    assert_eq!(kind, RoomKind::Channel);
    let (bmsgs, _) = bob.pull(&broom, None, None).await.unwrap();
    assert!(texts(&bmsgs).iter().any(|t| t.contains("designing the auth flow")));

    // Carol joins the SAME key → also caught up (N agents share one session, multi-use key).
    let mut carol = agent(&hub, "carol", None).await;
    carol.join(&inv.code).await.unwrap();
    let (cmsgs, _) = carol.pull(&inv.room, None, None).await.unwrap();
    assert!(texts(&cmsgs).iter().any(|t| t.contains("designing the auth flow")));

    // Bob replies; Alice sees it on her next pull (the conversation is shared, not peer-to-peer).
    bob.send_text(Target::Room { room: broom }, "got it — I'll take the token refresh").await.unwrap();
    let (amsgs, _) = alice.pull(&inv.room, None, None).await.unwrap();
    assert!(texts(&amsgs).iter().any(|t| t == "got it — I'll take the token refresh"));
}

#[tokio::test]
async fn session_key_joins_via_full_link() {
    // A joiner can paste the full invite link (parler://host/join/CODE), not just the bare code —
    // the hub normalizes it on redeem. This is what `parler_join_session` / `parler session join`
    // accept.
    let hub = start_hub().await;
    let mut alice = agent(&hub, "alice", None).await;
    let mut bob = agent(&hub, "bob", None).await;

    let inv = alice.invite(RoomKind::Channel, Some("design".into()), None, None).await.unwrap();
    assert!(inv.url.contains("/join/"));
    let (room, _kind) = bob.join(&inv.url).await.unwrap();
    assert_eq!(room, inv.room);
}

#[tokio::test]
async fn session_catchup_advances_cursor_without_duplicates() {
    // join_session pulls with since=None, which advances the cursor to the live edge — so after the
    // one-shot catch-up a later pull returns only genuinely new messages, never the backlog again.
    let hub = start_hub().await;
    let mut alice = agent(&hub, "alice", None).await;
    let mut bob = agent(&hub, "bob", None).await;
    let mut carol = agent(&hub, "carol", None).await;

    let inv = alice.invite(RoomKind::Channel, Some("design".into()), None, None).await.unwrap();
    alice.send_text(Target::Room { room: inv.room.clone() }, "seed context").await.unwrap();

    bob.join(&inv.code).await.unwrap();
    let (backlog, _) = bob.pull(&inv.room, None, None).await.unwrap(); // catch-up, advances cursor
    assert!(texts(&backlog).iter().any(|t| t == "seed context"));

    carol.join(&inv.code).await.unwrap();
    carol.send_text(Target::Room { room: inv.room.clone() }, "new note").await.unwrap();

    let (delta, _) = bob.pull(&inv.room, None, None).await.unwrap(); // only what arrived since
    assert_eq!(texts(&delta), vec!["new note"]);
}

#[tokio::test]
async fn invite_max_uses_is_enforced() {
    // A single-use key admits one joiner; a second redemption is refused.
    let hub = start_hub().await;
    let mut alice = agent(&hub, "alice", None).await;
    let mut bob = agent(&hub, "bob", None).await;
    let mut carol = agent(&hub, "carol", None).await;

    let inv = alice.invite(RoomKind::Channel, Some("oneshot".into()), None, Some(1)).await.unwrap();
    bob.join(&inv.code).await.unwrap(); // first redemption: ok
    assert!(carol.join(&inv.code).await.is_err()); // exhausted
}

#[tokio::test]
async fn approval_gated_session_requires_owner_consent() {
    // The security-critical flow: an approval-gated key lets an agent only *ask* to join — it cannot
    // read the conversation until the room owner approves. A leaked key alone grants nothing.
    let hub = start_hub().await;
    let mut alice = agent(&hub, "alice", Some("planner")).await;

    let inv = alice
        .invite_with_approval(RoomKind::Channel, Some("secret".into()), None, None, true)
        .await
        .unwrap();
    alice
        .send_text(Target::Room { room: inv.room.clone() }, "📋 context: launch code is 1234")
        .await
        .unwrap();

    // Bob redeems → pending, NOT joined; and he cannot read the room (the context stays hidden).
    let mut bob = agent(&hub, "bob", None).await;
    match bob.redeem(&inv.code).await.unwrap() {
        JoinOutcome::Pending { room } => assert_eq!(room, inv.room),
        JoinOutcome::Joined { .. } => panic!("an approval-gated redeem must not join outright"),
    }
    assert!(bob.pull(&inv.room, None, None).await.is_err(), "a pending joiner can't read the room");
    // The convenience `join()` surfaces the pending state as an error rather than a silent no-op.
    assert!(bob.join(&inv.code).await.is_err());
    // Re-redeeming while pending is idempotent (still pending).
    assert!(matches!(bob.redeem(&inv.code).await.unwrap(), JoinOutcome::Pending { .. }));

    // Only the owner can see/resolve the queue: the requester and a stranger are both refused.
    assert!(bob.join_requests(&inv.room).await.is_err());
    let mut eve = agent(&hub, "eve", None).await;
    assert!(eve.resolve_join(&inv.room, &bob.id, true).await.is_err(), "a non-owner cannot approve");

    let reqs = alice.join_requests(&inv.room).await.unwrap();
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].agent, bob.id);

    // Alice approves → bob is admitted and can finally read the seeded context.
    assert!(alice.resolve_join(&inv.room, &bob.id, true).await.unwrap());
    let room = match bob.redeem(&inv.code).await.unwrap() {
        JoinOutcome::Joined { room, .. } => room,
        JoinOutcome::Pending { .. } => panic!("bob should be admitted after approval"),
    };
    let (msgs, _) = bob.pull(&room, None, None).await.unwrap();
    assert!(texts(&msgs).iter().any(|t| t.contains("launch code is 1234")));
    assert!(alice.join_requests(&inv.room).await.unwrap().is_empty(), "the queue clears on approval");
}

#[tokio::test]
async fn denied_join_request_is_terminal_e2e() {
    // A denied requester is turned away for good: it can't read the room and can't re-request its way in.
    let hub = start_hub().await;
    let mut alice = agent(&hub, "alice", None).await;
    let inv = alice
        .invite_with_approval(RoomKind::Channel, Some("secret".into()), None, None, true)
        .await
        .unwrap();

    let mut eve = agent(&hub, "eve", None).await;
    assert!(matches!(eve.redeem(&inv.code).await.unwrap(), JoinOutcome::Pending { .. }));
    assert!(!alice.resolve_join(&inv.room, &eve.id, false).await.unwrap()); // deny

    assert!(eve.pull(&inv.room, None, None).await.is_err());
    assert!(eve.redeem(&inv.code).await.is_err());
}

#[tokio::test]
async fn approval_gate_not_bypassable_by_reinviting_to_the_room() {
    // Regression for the gate's whole point: a non-member must not be able to walk past approval by
    // minting its OWN invite for the same (guessable, topic-named) room and self-joining. Eve here
    // doesn't even use the key — she only knows the topic.
    let hub = start_hub().await;
    let mut alice = agent(&hub, "alice", None).await;
    let inv = alice
        .invite_with_approval(RoomKind::Channel, Some("auth-redesign".into()), None, None, true)
        .await
        .unwrap();
    alice
        .send_text(Target::Room { room: inv.room.clone() }, "secret: launch code 1234")
        .await
        .unwrap();

    let mut eve = agent(&hub, "eve", None).await;
    // Self-adding via an invite to the same existing room is refused…
    assert!(
        eve.invite(RoomKind::Channel, Some("auth-redesign".into()), None, None).await.is_err(),
        "a non-member must not be able to invite itself into an existing room"
    );
    // …so eve is not a member and cannot read the seeded context.
    assert!(eve.pull(&inv.room, None, None).await.is_err(), "the approval gate holds");

    // The legitimate owner can still mint further invites for its own room.
    assert!(alice.invite(RoomKind::Channel, Some("auth-redesign".into()), None, None).await.is_ok());
}

#[tokio::test]
async fn ordinary_session_key_still_joins_without_approval() {
    // Backward-compat: a key minted without approval (the historical default) joins on the spot.
    let hub = start_hub().await;
    let mut alice = agent(&hub, "alice", None).await;
    let mut bob = agent(&hub, "bob", None).await;

    let inv = alice.invite(RoomKind::Channel, Some("open".into()), None, None).await.unwrap();
    match bob.redeem(&inv.code).await.unwrap() {
        JoinOutcome::Joined { room, .. } => assert_eq!(room, inv.room),
        JoinOutcome::Pending { .. } => panic!("an ungated key should join immediately"),
    }
    // And the owner's request queue is empty (nothing is gated).
    assert!(alice.join_requests(&inv.room).await.unwrap().is_empty());
}

#[tokio::test]
async fn expired_invite_is_rejected() {
    // A key with ttl=0 is already expired by the time anyone tries to redeem it.
    let hub = start_hub().await;
    let mut alice = agent(&hub, "alice", None).await;
    let mut bob = agent(&hub, "bob", None).await;

    let inv = alice.invite(RoomKind::Channel, Some("ephemeral".into()), Some(0), None).await.unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(bob.join(&inv.code).await.is_err());
}

// ---- web session viewer (read-only watch tokens) ----

/// A minimal async HTTP/1.1 GET with an optional `Authorization: Bearer`. Returns `(status, body)`.
/// The session viewer is plain HTTP (a browser, not an agent), so we exercise it over a real socket —
/// the same dependency-free client style as the hub's smoke test.
async fn http_get(hub_ws: &str, path: &str, bearer: Option<&str>) -> (u16, String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let addr = hub_ws.strip_prefix("ws://").expect("ws url");
    let mut stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
    let auth = bearer.map(|t| format!("Authorization: Bearer {t}\r\n")).unwrap_or_default();
    let req = format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\n{auth}Connection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).await.expect("write");
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.expect("read");
    let raw = String::from_utf8_lossy(&buf).into_owned();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((raw.as_str(), ""));
    let status = head
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    (status, body.to_string())
}

#[tokio::test]
async fn web_session_viewer_reads_a_watched_session() {
    let hub = start_hub().await;
    let mut alice = agent(&hub, "alice", Some("planner")).await;

    // Alice opens an approval-gated session (the website-monitored case) and seeds it with context.
    let inv = alice
        .invite_with_approval(RoomKind::Channel, Some("design".into()), None, None, true)
        .await
        .unwrap();
    alice
        .send_text(
            Target::Room { room: inv.room.clone() },
            "context: redesigning auth, see src/auth.rs",
        )
        .await
        .unwrap();

    // Bob asks to join; Alice approves — now two agents are in the room.
    let mut bob = agent(&hub, "bob", Some("reviewer")).await;
    match bob.redeem(&inv.code).await.unwrap() {
        JoinOutcome::Pending { .. } => {}
        JoinOutcome::Joined { .. } => panic!("an approval-gated session should hold the joiner pending"),
    }
    alice.resolve_join(&inv.room, &bob.id, true).await.unwrap();

    // The owner mints a read-only watch code for the website.
    let (watch, _exp) = alice.mint_watch_token(&inv.room, None).await.unwrap();

    // The viewer endpoint returns the conversation + the agent count, gated by the watch token.
    let (status, body) = http_get(&hub, "/api/session", Some(&watch)).await;
    assert_eq!(status, 200, "watch token authorizes the viewer; body={body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["memberCount"], 2, "two agents are in the room");
    assert_eq!(v["onlineCount"], 2);
    assert_eq!(v["room"], inv.room);
    let names: Vec<&str> =
        v["agents"].as_array().unwrap().iter().map(|a| a["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"alice") && names.contains(&"bob"), "roster lists both agents: {names:?}");
    let text = v["messages"][0]["parts"][0]["text"].as_str().unwrap();
    assert!(text.contains("redesigning auth"), "the conversation content is visible: {text}");
    // The viewer shape must not leak agent ids (public keys).
    assert!(v["agents"][0].get("id").is_none(), "the viewer must not expose agent ids");

    // Activity metrics: estimated tokens spent + a per-agent breakdown (the new insight surface).
    let stats = &v["stats"];
    assert!(stats["messages"].as_i64().unwrap() >= 1, "at least alice's context message is counted");
    assert!(stats["estimatedTokens"].as_i64().unwrap() > 0, "the conversation has a non-zero token estimate");
    assert!(stats["firstMessageAt"].as_i64().is_some(), "the activity span has a start");
    assert!(stats["lastMessageAt"].as_i64().is_some(), "the activity span has an end");
    let per_agent = stats["perAgent"].as_array().unwrap();
    let alice_row = per_agent.iter().find(|a| a["name"] == "alice").expect("alice is in the breakdown");
    assert!(alice_row["estimatedTokens"].as_i64().unwrap() > 0, "alice's token spend is attributed to her");
    assert!(alice_row["messages"].as_i64().unwrap() >= 1);
    // The per-agent breakdown is display-identity only — no id leaks here either.
    assert!(alice_row.get("id").is_none(), "the per-agent breakdown must not expose agent ids");

    // Incremental poll: nothing new past the cursor.
    let cursor = v["cursor"].as_i64().unwrap();
    let (_s, body2) = http_get(&hub, &format!("/api/session?since={cursor}"), Some(&watch)).await;
    let v2: serde_json::Value = serde_json::from_str(&body2).unwrap();
    assert_eq!(v2["messages"].as_array().unwrap().len(), 0, "no messages newer than the cursor");

    // Security: the *join key* is NOT a watch token — it can't read the conversation from the web.
    let (status, _b) = http_get(&hub, "/api/session", Some(&inv.code)).await;
    assert_eq!(status, 401, "the approval-gated join key can't read the session over REST");

    // A bogus or absent token is refused.
    assert_eq!(http_get(&hub, "/api/session", Some("NOPE")).await.0, 401);
    assert_eq!(http_get(&hub, "/api/session", None).await.0, 401);
}

#[tokio::test]
async fn only_the_session_owner_can_mint_a_watch_code() {
    let hub = start_hub().await;
    let mut alice = agent(&hub, "alice", None).await;
    let inv = alice
        .invite_with_approval(RoomKind::Channel, Some("design".into()), None, None, false)
        .await
        .unwrap();

    // Bob joins (open key), so he's a member — but not the owner.
    let mut bob = agent(&hub, "bob", None).await;
    bob.join(&inv.code).await.unwrap();

    // A non-owner member cannot expose the session to outside viewers.
    assert!(bob.mint_watch_token(&inv.room, None).await.is_err(), "only the owner may mint a watch code");
    // The owner can.
    assert!(alice.mint_watch_token(&inv.room, None).await.is_ok());
}

// ---- idle auto-disconnect + transparent reconnect ----

#[tokio::test]
async fn idle_authenticated_connection_is_dropped_by_the_hub() {
    // The hub frees an authenticated connection that stays silent past the idle bound. We check on a
    // raw transport — below MeshAgent's transparent reconnect — to observe the drop itself.
    let hub = start_hub_with_idle(Some(Duration::from_millis(200))).await;
    let id = cfg(&hub, "alice", None);
    let mut client = HubClient::connect(&id.hub_url, &id.identity, &id.name, None).await.unwrap();
    client.request(ClientFrame::Rooms).await.expect("connected");

    // Stay silent well past the timeout; the next raw request finds the socket gone.
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(client.request(ClientFrame::Rooms).await.is_err());
}

#[tokio::test]
async fn idle_timeout_resets_on_activity() {
    // The idle deadline is measured from the last received frame, so a client that keeps acting
    // (gaps shorter than the timeout) stays connected — then is dropped once it goes quiet. Checked
    // on a raw connection so MeshAgent's reconnect doesn't mask the eventual drop.
    let hub = start_hub_with_idle(Some(Duration::from_millis(400))).await;
    let id = cfg(&hub, "alice", None);
    let mut client = HubClient::connect(&id.hub_url, &id.identity, &id.name, None).await.unwrap();

    for _ in 0..4 {
        tokio::time::sleep(Duration::from_millis(150)).await; // < 400ms: resets the deadline
        client.request(ClientFrame::Rooms).await.expect("still connected while active");
    }

    tokio::time::sleep(Duration::from_millis(700)).await; // now go silent past the timeout
    assert!(client.request(ClientFrame::Rooms).await.is_err());
}

#[tokio::test]
async fn idle_timeout_none_keeps_connection_open() {
    // With the idle timeout disabled, a silent connection survives.
    let hub = start_hub_with_idle(None).await;
    let mut alice = agent(&hub, "alice", None).await;

    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(alice.rooms().await.is_ok());
}

#[tokio::test]
async fn idle_drop_is_transparently_reconnected_and_resumes() {
    // A quiet teammate whose connection the hub reaps for idleness is silently re-dialed on their
    // next action — no error surfaced, still in the room, resuming from the durable cursor with no
    // re-approval. This is what keeps a team's shared session alive across lulls.
    let hub = start_hub_with_idle(Some(Duration::from_millis(200))).await;
    let mut alice = agent(&hub, "alice", None).await;
    let mut bob = MeshAgent::connect(&cfg(&hub, "bob", None)).await.unwrap();
    bob.subscribe().await.ok(); // like the MCP server: subscribed for push

    let inv = alice.invite(RoomKind::Channel, Some("team".into()), None, None).await.unwrap();
    let room = inv.room.clone();
    bob.join(&inv.code).await.unwrap();
    bob.pull(&room, None, None).await.unwrap(); // catch up, cursor at the live edge

    // Both go quiet long enough for the hub to reap their sockets.
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Alice posts (reconnecting transparently herself), then bob's next pull transparently
    // reconnects and returns the message he'd otherwise have missed — he never left the room.
    alice.send_text(Target::Room { room: room.clone() }, "while you were away").await.unwrap();
    let (m, _) = bob.pull(&room, None, None).await.expect("bob transparently reconnects");
    assert_eq!(texts(&m), vec!["while you were away"]);

    // And he can still send into the same session after the reconnect.
    bob.send_text(Target::Room { room: room.clone() }, "back now").await.unwrap();
    let (m2, _) = alice.pull(&room, None, None).await.unwrap();
    assert!(texts(&m2).contains(&"back now".to_string()));
}

// ---- authenticated messages (the hub relays, but can't forge or alter what an agent said) ----

fn status(m: &StoredMessage) -> SigStatus {
    verify_message(&m.from.id, &m.parts, m.reply_to.as_deref())
}

#[tokio::test]
async fn signed_channel_message_verifies_after_a_real_round_trip() {
    let hub = start_hub().await;
    let mut alice = agent(&hub, "alice", Some("planner")).await;
    let mut bob = agent(&hub, "bob", None).await;

    let inv = alice.invite(RoomKind::Channel, Some("team".into()), None, None).await.unwrap();
    bob.join(&inv.code).await.unwrap();

    alice.send_text(Target::Room { room: inv.room.clone() }, "ship it").await.unwrap();
    let (msgs, _) = bob.pull(&inv.room, None, None).await.unwrap();

    // The content is intact (the signature part doesn't pollute the rendered text)…
    assert_eq!(texts(&msgs), vec!["ship it"]);
    // …and it verifies against alice's own id: the hub relayed it, it did not author it.
    assert_eq!(msgs[0].from.id, alice.id);
    assert_eq!(status(&msgs[0]), SigStatus::Valid);
}

#[tokio::test]
async fn signed_dm_verifies() {
    let hub = start_hub().await;
    let mut alice = agent(&hub, "alice", None).await;
    let mut bob = agent(&hub, "bob", None).await;

    let inv = alice.invite(RoomKind::Dm, None, None, None).await.unwrap();
    let (broom, _) = bob.join(&inv.code).await.unwrap();
    alice.send_text(Target::Dm { agent: bob.id.clone() }, "secret plan").await.unwrap();

    let (m, _) = bob.pull(&broom, None, None).await.unwrap();
    assert_eq!(status(&m[0]), SigStatus::Valid);
}

#[tokio::test]
async fn a_tampered_or_forged_message_is_detected() {
    let hub = start_hub().await;
    let mut alice = agent(&hub, "alice", None).await;
    let mut bob = agent(&hub, "bob", None).await;

    let inv = alice.invite(RoomKind::Channel, Some("team".into()), None, None).await.unwrap();
    bob.join(&inv.code).await.unwrap();
    alice.send_text(Target::Room { room: inv.room.clone() }, "deploy v1").await.unwrap();

    let (msgs, _) = bob.pull(&inv.room, None, None).await.unwrap();
    let genuine = msgs[0].clone();
    assert_eq!(status(&genuine), SigStatus::Valid);

    // (1) A malicious hub rewrites the authored content → signature no longer matches.
    let mut altered = genuine.clone();
    altered.parts = altered
        .parts
        .iter()
        .map(|p| match p {
            Part::Text(_) => Part::text("deploy v1 to prod"),
            other => other.clone(),
        })
        .collect();
    assert_eq!(status(&altered), SigStatus::Invalid);

    // (2) A malicious hub re-attributes alice's signature to bob → fails under bob's key.
    let mut forged = genuine.clone();
    forged.from.id = bob.id.clone();
    assert_eq!(status(&forged), SigStatus::Invalid);
}

#[tokio::test]
async fn a_pushed_delivery_is_also_verifiable() {
    let hub = start_hub().await;
    let mut alice = agent(&hub, "alice", None).await;
    let mut bob = agent(&hub, "bob", None).await;

    let inv = alice.invite(RoomKind::Channel, Some("team".into()), None, None).await.unwrap();
    bob.join(&inv.code).await.unwrap();
    assert!(bob.subscribe().await.unwrap(), "hub should support push");

    alice.send_text(Target::Room { room: inv.room.clone() }, "live ping").await.unwrap();
    let pushed = bob
        .next_delivery(Duration::from_secs(5))
        .await
        .unwrap()
        .expect("a pushed delivery within the window");
    assert_eq!(status(&pushed), SigStatus::Valid);
    assert_eq!(texts(std::slice::from_ref(&pushed)), vec!["live ping"]);
}

// ---- server-side Pull wait + client heartbeat (issues #90 / #87) ----

#[tokio::test]
async fn pull_wait_returns_early_when_a_peer_message_lands() {
    // #90 core: `pull_wait` parks on the hub and completes the moment a peer sends — not after the
    // full window. The connection is healthy either way.
    let hub = start_hub().await;
    let mut alice = agent(&hub, "alice", None).await;
    let mut bob = agent(&hub, "bob", None).await;
    let inv = alice.invite(RoomKind::Channel, Some("wait".into()), None, None).await.unwrap();
    bob.join(&inv.code).await.unwrap();

    // Bob long-polls up to 10s; alice sends after 150ms. Bob must wake well before the window.
    let send = async {
        tokio::time::sleep(Duration::from_millis(150)).await;
        alice.send_text(Target::Room { room: inv.room.clone() }, "woke you").await.unwrap();
    };
    let recv = async {
        let started = std::time::Instant::now();
        let (msgs, _cursor, waited) = bob.pull_wait(&inv.room, None, 10).await.unwrap();
        (msgs, waited, started.elapsed())
    };
    let (_s, (msgs, waited, elapsed)) = tokio::join!(send, recv);
    assert_eq!(texts(&msgs), vec!["woke you"], "the parked pull returns the peer message");
    assert!(waited, "a real server-side wait occurred (the hub parked the request)");
    assert!(elapsed < Duration::from_secs(5), "woke on the message, not at the timeout: {elapsed:?}");
}

#[tokio::test]
async fn pull_wait_is_empty_at_timeout_and_connection_stays_healthy() {
    // #90: an empty room returns an empty batch at the deadline (no hang, no error), and the same
    // connection keeps working afterward.
    let hub = start_hub().await;
    let mut alice = agent(&hub, "alice", None).await;
    let mut bob = agent(&hub, "bob", None).await;
    let inv = alice.invite(RoomKind::Channel, Some("quiet".into()), None, None).await.unwrap();
    bob.join(&inv.code).await.unwrap();

    let started = std::time::Instant::now();
    let (msgs, _cursor, _waited) = bob.pull_wait(&inv.room, None, 1).await.unwrap();
    assert!(msgs.is_empty(), "nothing sent → empty at timeout");
    assert!(started.elapsed() >= Duration::from_secs(1), "it waited out the window");

    // The connection is still healthy: an ordinary send + pull works right after the wait.
    alice.send_text(Target::Room { room: inv.room.clone() }, "after wait").await.unwrap();
    let (after, _) = bob.pull(&inv.room, None, None).await.unwrap();
    assert_eq!(texts(&after), vec!["after wait"]);
}

#[tokio::test]
async fn pull_wait_works_without_a_push_subscription() {
    // #90 / #87: the previously-degraded mode. Bob NEVER subscribes, yet the server-side wait still
    // delivers a peer's message early — long-poll no longer depends on push machinery.
    let hub = start_hub().await;
    let mut alice = agent(&hub, "alice", None).await;
    let mut bob = agent(&hub, "bob", None).await;
    assert!(!bob.push_active(), "bob holds no push subscription");
    let inv = alice.invite(RoomKind::Channel, Some("nopush".into()), None, None).await.unwrap();
    bob.join(&inv.code).await.unwrap();

    let send = async {
        tokio::time::sleep(Duration::from_millis(150)).await;
        alice.send_text(Target::Room { room: inv.room.clone() }, "no push needed").await.unwrap();
    };
    let recv = async {
        let started = std::time::Instant::now();
        let (msgs, _c, waited) = bob.pull_wait(&inv.room, None, 10).await.unwrap();
        (msgs, waited, started.elapsed())
    };
    let (_s, (msgs, waited, elapsed)) = tokio::join!(send, recv);
    assert!(!bob.push_active(), "still no subscription — the wait was purely server-side");
    assert_eq!(texts(&msgs), vec!["no push needed"]);
    assert!(waited, "the hub parked the request even with no subscription");
    assert!(elapsed < Duration::from_secs(5), "woke on the message: {elapsed:?}");
}

#[tokio::test]
async fn pull_wait_never_advances_the_cursor_past_the_returned_batch() {
    // The load-bearing invariant: a server-side wait resolves through normal Pull/cursor semantics —
    // it advances the cursor only through the batch it returns. A wait that times out empty must not
    // move the cursor, so a message sent *after* it is still delivered by the next pull.
    let hub = start_hub().await;
    let mut alice = agent(&hub, "alice", None).await;
    let mut bob = agent(&hub, "bob", None).await;
    let inv = alice.invite(RoomKind::Channel, Some("cursor".into()), None, None).await.unwrap();
    bob.join(&inv.code).await.unwrap();

    // An empty wait: cursor must stay put.
    let (empty, _c, _w) = bob.pull_wait(&inv.room, None, 1).await.unwrap();
    assert!(empty.is_empty());
    // Now alice sends two; a plain pull returns BOTH (proof the empty wait didn't skip the cursor
    // ahead of them).
    alice.send_text(Target::Room { room: inv.room.clone() }, "one").await.unwrap();
    alice.send_text(Target::Room { room: inv.room.clone() }, "two").await.unwrap();
    let (got, _) = bob.pull(&inv.room, None, None).await.unwrap();
    assert_eq!(texts(&got), vec!["one", "two"], "the empty wait left the cursor untouched");
}

#[tokio::test]
async fn plain_pull_without_wait_is_unchanged() {
    // Backward-compat: a `Pull` with no `wait_secs` behaves exactly as before — immediate reply,
    // empty when the room is empty, no parking.
    let hub = start_hub().await;
    let mut alice = agent(&hub, "alice", None).await;
    let mut bob = agent(&hub, "bob", None).await;
    let inv = alice.invite(RoomKind::Channel, Some("plain".into()), None, None).await.unwrap();
    bob.join(&inv.code).await.unwrap();

    let started = std::time::Instant::now();
    let (msgs, _c) = bob.pull(&inv.room, None, None).await.unwrap();
    assert!(msgs.is_empty());
    assert!(started.elapsed() < Duration::from_millis(500), "a plain pull returns immediately");
}

#[tokio::test]
async fn heartbeat_reconnects_a_half_open_transport_during_a_long_poll() {
    // #87: a half-open transport (the proxy silently dropped the socket; reads never complete) is
    // detected within the heartbeat window and transparently reconnected — no error to the caller,
    // and the next recv works. We simulate half-open with a transport whose FIRST request hangs
    // forever; the heartbeat times out on it, forces a reconnect (which rebuilds a live client), and
    // the resumed pull succeeds.
    let hub = start_hub().await;
    // Alice sets up a room and leaves a message waiting, so the resumed connection has something to
    // pull once bob heals.
    let mut alice = agent(&hub, "alice", None).await;
    let inv = alice.invite(RoomKind::Channel, Some("halfopen".into()), None, None).await.unwrap();

    // Build bob over a real client wrapped in an (initially disarmed) fault-injecting transport.
    let bob_cfg = cfg(&hub, "bob", None);
    let real = HubClient::connect(&bob_cfg.hub_url, &bob_cfg.identity, &bob_cfg.name, None)
        .await
        .unwrap();
    let (flaky, armed) = FlakyTransport::new(Box::new(real));
    let mut bob = MeshAgent::with_transport_and_identity(
        Box::new(flaky),
        bob_cfg.identity.clone(),
        bob_cfg.name.clone(),
        None,
        bob_cfg.hub_url.clone(),
    );
    bob.join(&inv.code).await.unwrap(); // setup runs on a healthy transport
    alice.send_text(Target::Room { room: inv.room.clone() }, "survived the drop").await.unwrap();

    // Now go half-open right before the long-poll: the next request (the heartbeat's ping) hangs.
    armed.store(true, std::sync::atomic::Ordering::SeqCst);

    // The long-poll's heartbeat times out on the half-open socket, reconnects transparently, and the
    // resumed pull returns the waiting message — the caller sees no error. (Bounded well under the
    // wait window: the 5s heartbeat timeout + a fresh dial.)
    let started = std::time::Instant::now();
    let (msgs, _c, _w) = bob.pull_wait(&inv.room, None, 30).await.unwrap();
    assert_eq!(texts(&msgs), vec!["survived the drop"], "the half-open drop was healed transparently");
    assert!(started.elapsed() < Duration::from_secs(20), "healed within the heartbeat window");
}

#[tokio::test]
async fn an_unsigned_legacy_message_is_flagged_not_trusted() {
    // A message with no com.parler.sig part — an older client, or a hub fabricating one from nothing —
    // is reported Unsigned (surfaced as ⚠), never silently treated as authentic.
    let m = StoredMessage {
        seq: 1,
        id: "x".into(),
        room: "team".into(),
        from: EndpointRef { id: "UGHOST".into(), name: "ghost".into(), role: None },
        parts: vec![Part::text("trust me")],
        mentions: None,
        reply_to: None,
        ts: 1,
    };
    assert_eq!(status(&m), SigStatus::Unsigned);
}
