//! End-to-end: a real in-process hub + real WebSocket clients exercising the whole feature —
//! the three delivery patterns, paste-a-code pairing, memory scoping, durable resume, and authz.

use parler_connector::{Config, MeshAgent};
use parler_protocol::{Part, RoomKind, StoredMessage, Target};
use std::sync::Arc;

/// Start an in-memory hub on an ephemeral port; return its ws:// URL.
async fn start_hub() -> String {
    let store = parler_hub::Store::open(None).unwrap();
    let state = Arc::new(parler_hub::HubState {
        store,
        public_url: "parler://test".into(),
    });
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
async fn non_member_cannot_read_a_room() {
    let hub = start_hub().await;
    let mut alice = agent(&hub, "alice", None).await;
    let mut eve = agent(&hub, "eve", None).await;

    let inv = alice.invite(RoomKind::Channel, Some("secret".into()), None, None).await.unwrap();
    alice.send_text(Target::Room { room: inv.room.clone() }, "classified").await.unwrap();

    // eve never redeemed the invite → not a member → reads are refused.
    assert!(eve.pull(&inv.room, None, None).await.is_err());
}
