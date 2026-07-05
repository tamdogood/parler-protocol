//! End-to-end discovery: a real in-process hub + WebSocket clients exercising the directory —
//! public vs hub scope visibility, signed-card verification, forged/tampered-card rejection,
//! and directory-token minting.

use parler_connector::{Config, HubClient, MeshAgent, MeshTransport};
use parler_protocol::{
    canonical_card_bytes, AgentCard, ClientFrame, DiscoverScope, EndpointKind, Part, ServerFrame,
    StoredMessage, Target, Visibility,
};
use std::sync::Arc;

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

async fn start_hub(public: bool) -> String {
    let store = parler_hub::Store::open(None).unwrap();
    let mode = if public {
        parler_hub::HubMode::Public
    } else {
        parler_hub::HubMode::Private
    };
    let state = Arc::new(parler_hub::HubState::new(
        store,
        "parler://test".into(),
        "Parler Protocol Public".into(),
        mode,
    ));
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

fn bare_card(id: &str, name: &str) -> AgentCard {
    AgentCard {
        id: id.into(),
        name: name.into(),
        kind: EndpointKind::Agent,
        role: None,
        description: None,
        tags: None,
        skills: None,
        meta: None,
        protocol_version: None,
    }
}

#[tokio::test]
async fn public_and_hub_scope_visibility() {
    let hub = start_hub(true).await;
    let mut alice = agent(&hub, "alice", Some("planner")).await;
    let mut bob = agent(&hub, "bob", Some("reviewer")).await;

    // alice opts public + signs; bob stays private.
    let (vis, verified) = alice
        .register(Visibility::Public, vec!["planning".into()], vec![], Some("plans sprints".into()))
        .await
        .unwrap();
    assert_eq!(vis, Visibility::Public);
    assert!(verified, "a card registered through MeshAgent is always signed + verified");
    bob.register(Visibility::Private, vec!["review".into()], vec![], None).await.unwrap();

    // Public scope leaks only the public agent.
    let pubd = bob.discover(DiscoverScope::Public, None, None, None, None, None).await.unwrap();
    assert_eq!(pubd.len(), 1);
    assert_eq!(pubd[0].card.name, "alice");
    assert!(pubd[0].verified);
    assert_eq!(pubd[0].hub, "Parler Protocol Public");

    // Hub scope (same-hub view) returns both.
    let hubd = alice.discover(DiscoverScope::Hub, None, None, None, None, None).await.unwrap();
    assert_eq!(hubd.len(), 2);

    // Tag filter narrows to bob; lookup resolves a peer's card.
    let by_tag = alice.discover(DiscoverScope::Hub, None, Some("review".into()), None, None, None).await.unwrap();
    assert_eq!(by_tag.len(), 1);
    assert_eq!(by_tag[0].card.name, "bob");

    let card = bob.lookup(&alice.id).await.unwrap().expect("alice's card");
    assert_eq!(card.card.name, "alice");
    assert!(card.verified);
}

#[tokio::test]
async fn forged_and_tampered_cards_are_rejected() {
    let hub = start_hub(true).await;
    let me = parler_auth::new_identity().unwrap();
    let mut raw = HubClient::connect(&hub, &me, "mallory", None).await.unwrap();
    let card = bare_card(&me.id, "mallory");

    // (a) A present-but-garbage signature is refused outright.
    let bad = ClientFrame::Register {
        card: card.clone(),
        visibility: Visibility::Public,
        sig: Some("AAAA".into()),
    };
    assert!(raw.request(bad).await.is_err(), "garbage signature must be rejected");

    // (b) A card claiming someone else's id is refused (even if validly self-signed).
    let victim = parler_auth::new_identity().unwrap();
    let mut spoof = card.clone();
    spoof.id = victim.id.clone();
    let spoof_sig = parler_auth::sign(&me.seed, &canonical_card_bytes(&spoof)).unwrap();
    let forged = ClientFrame::Register { card: spoof, visibility: Visibility::Public, sig: Some(spoof_sig) };
    assert!(raw.request(forged).await.is_err(), "id-spoofed card must be rejected");

    // (c) A correctly self-signed card for my own id is accepted + verified.
    let good_sig = parler_auth::sign(&me.seed, &canonical_card_bytes(&card)).unwrap();
    let ok = ClientFrame::Register { card: card.clone(), visibility: Visibility::Public, sig: Some(good_sig) };
    match raw.request(ok).await.unwrap() {
        ServerFrame::Registered { verified, .. } => assert!(verified),
        other => panic!("unexpected reply: {other:?}"),
    }

    // (d) An unsigned card is allowed but marked unverified.
    let unsigned = ClientFrame::Register { card, visibility: Visibility::Private, sig: None };
    match raw.request(unsigned).await.unwrap() {
        ServerFrame::Registered { verified, .. } => assert!(!verified),
        other => panic!("unexpected reply: {other:?}"),
    }
}

#[tokio::test]
async fn mints_a_directory_token() {
    let hub = start_hub(false).await; // a private hub
    let mut carol = agent(&hub, "carol", None).await;
    let (token, expires_at) = carol.mint_directory_token(Some(120)).await.unwrap();
    assert!(token.len() >= 16, "tokens are high-entropy");
    assert!(expires_at > 0);
}

#[tokio::test]
async fn discovered_agent_can_be_dmed_without_pairing() {
    let hub = start_hub(true).await;
    let mut alice = agent(&hub, "alice", None).await;
    let mut bob = agent(&hub, "bob", None).await;

    // bob publishes a card → he becomes reachable in the directory.
    bob.register(Visibility::Public, vec!["research".into()], vec![], None).await.unwrap();

    // alice finds bob and DMs him by id with NO prior invite/redeem — the hub opens the DM room.
    let (_id, _seq, room) = alice
        .send_text(Target::Dm { agent: bob.id.clone() }, "found you in the directory")
        .await
        .unwrap();

    // bob sees the new DM room and the message.
    let rooms = bob.rooms().await.unwrap();
    assert!(rooms.iter().any(|r| r.name == room), "bob is a member of the new DM room");
    let (msgs, _) = bob.pull(&room, None, None).await.unwrap();
    assert_eq!(texts(&msgs), vec!["found you in the directory"]);

    // bob replies by id; alice receives it on the same room — a live round-trip.
    bob.send_text(Target::Dm { agent: alice.id.clone() }, "hi alice").await.unwrap();
    let (back, _) = alice.pull(&room, None, None).await.unwrap();
    assert!(texts(&back).contains(&"hi alice".to_string()));
}

#[tokio::test]
async fn undiscoverable_agent_still_requires_pairing() {
    let hub = start_hub(true).await;
    let mut alice = agent(&hub, "alice", None).await;
    let carol = agent(&hub, "carol", None).await; // connects but never registers a card

    // carol has no directory card, so she can't be cold-DMed — pairing is still required.
    assert!(
        alice.send_text(Target::Dm { agent: carol.id.clone() }, "hi").await.is_err(),
        "an undiscoverable agent requires invite/redeem"
    );
}
