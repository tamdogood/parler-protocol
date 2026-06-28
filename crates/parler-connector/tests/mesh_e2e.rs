//! End-to-end: a real in-process hub + real WebSocket clients exercising the whole feature —
//! the three delivery patterns, paste-a-code pairing, memory scoping, durable resume, and authz.

use parler_connector::{BundleMeta, Config, MeshAgent};
use parler_protocol::{BundleRef, Part, RoomKind, StoredMessage, Target};
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
    alice.remember("the deploy strategy is blue-green", None, Some(inv.room.clone())).await.unwrap();
    let hits = bob.recall("deploy", Some(inv.room.clone()), None).await.unwrap();
    assert_eq!(hits.len(), 1);
    assert!(hits[0].text.contains("blue-green"));

    // A private fact is only recallable by its author.
    alice.remember("my api token is xyz", Some("token".into()), None).await.unwrap();
    assert_eq!(bob.recall("token", None, None).await.unwrap().len(), 0);
    assert_eq!(alice.recall("token", None, None).await.unwrap().len(), 1);
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
        assert_eq!(texts(&m), vec!["first"]); // cursor now advanced past "first"
    } // bob disconnects

    alice.send_text(Target::Room { room: room.clone() }, "second").await.unwrap();

    // A fresh connection for the same identity resumes from the durable cursor.
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
async fn expired_invite_is_rejected() {
    // A key with ttl=0 is already expired by the time anyone tries to redeem it.
    let hub = start_hub().await;
    let mut alice = agent(&hub, "alice", None).await;
    let mut bob = agent(&hub, "bob", None).await;

    let inv = alice.invite(RoomKind::Channel, Some("ephemeral".into()), Some(0), None).await.unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(bob.join(&inv.code).await.is_err());
}

// ---- idle auto-disconnect ----

#[tokio::test]
async fn idle_authenticated_connection_is_disconnected() {
    // A connection silent past the idle timeout is dropped by the hub.
    let hub = start_hub_with_idle(Some(Duration::from_millis(200))).await;
    let mut alice = agent(&hub, "alice", None).await;

    // Stay silent well past the timeout, then any request should fail (the hub closed the socket).
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(alice.rooms().await.is_err());
}

#[tokio::test]
async fn idle_timeout_resets_on_activity() {
    // The idle deadline is measured from the last received frame, so an agent that keeps acting
    // (gaps shorter than the timeout) stays connected — then is dropped once it goes quiet.
    let hub = start_hub_with_idle(Some(Duration::from_millis(400))).await;
    let mut alice = agent(&hub, "alice", None).await;

    for _ in 0..4 {
        tokio::time::sleep(Duration::from_millis(150)).await; // < 400ms: resets the deadline
        alice.rooms().await.expect("still connected while active");
    }

    tokio::time::sleep(Duration::from_millis(700)).await; // now go silent past the timeout
    assert!(alice.rooms().await.is_err());
}

#[tokio::test]
async fn idle_timeout_none_keeps_connection_open() {
    // With the idle timeout disabled, a silent connection survives.
    let hub = start_hub_with_idle(None).await;
    let mut alice = agent(&hub, "alice", None).await;

    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(alice.rooms().await.is_ok());
}
