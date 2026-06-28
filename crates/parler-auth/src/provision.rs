//! The provisioner — the signer capability for a space. Port of Cotal `provision.ts`.
//!
//! A space is one NATS *account*; every agent is a *user* in it. This module mints the
//! decentralized-JWT trust chain (operator → account → user), renders the `nats-server` config, and
//! builds the per-profile default-deny ACLs from the shared subject/stream/durable builders so the
//! grants can never drift from the wire layout.

use crate::error::AuthError;
use crate::identity::Identity;
use crate::jwt;
use nkeys::KeyPair;
use parler_protocol::{
    account_connect_subject, account_disconnect_subject, acl_bucket, anycast_subject,
    assert_valid_channel, channel_bucket, chat_hist_durable, chat_stream, chat_subject,
    chat_wildcard, connz_request_subject, control_service_subject, delivery_bucket, dlv_durable,
    dlv_stream, dm_durable, dm_stream, inbox_stream, members_bucket, membership_bucket,
    presence_bucket, space_prefix, task_durable, task_stream, token, unicast_subject,
    CONTROL_DELIVERY, CONTROL_PRIVILEGED, CONTROL_SELF_SERVICE, INBOX_READER_DURABLE,
    MEMBERSHIP_INBOX_PREFIX,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Credential profiles. `delivery`/`membership-rw` are server-side infra creds, never CLI-minted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    Agent,
    Observer,
    Admin,
    Manager,
    Delivery,
    MembershipRw,
}

impl Profile {
    pub fn as_str(self) -> &'static str {
        match self {
            Profile::Agent => "agent",
            Profile::Observer => "observer",
            Profile::Admin => "admin",
            Profile::Manager => "manager",
            Profile::Delivery => "delivery",
            Profile::MembershipRw => "membership-rw",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OperatorMaterial {
    pub seed: String,
    pub jwt: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AccountMaterial {
    #[serde(rename = "pub")]
    pub public: String,
    pub seed: String,
    pub jwt: String,
    #[serde(rename = "signingSeed")]
    pub signing_seed: String,
    #[serde(rename = "signingPub")]
    pub signing_pub: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SysMaterial {
    #[serde(rename = "pub")]
    pub public: String,
    pub jwt: String,
    /// In-memory only; never persisted (stripped by [`strip_space_auth`]).
    #[serde(rename = "signingSeed", default, skip_serializing_if = "Option::is_none")]
    pub signing_seed: Option<String>,
}

/// A space's persisted trust material. `account.signing_seed` is the sensitive provisioner secret.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpaceAuth {
    pub space: String,
    pub operator: OperatorMaterial,
    pub account: AccountMaterial,
    pub sys: SysMaterial,
}

/// Unlimited NATS/account limits plus unlimited JetStream storage (the DATA account).
fn data_limits() -> Value {
    json!({
        "subs": -1, "data": -1, "payload": -1, "imports": -1, "exports": -1,
        "wildcards": true, "conn": -1, "leaf": -1, "mem_storage": -1, "disk_storage": -1
    })
}

/// Unlimited NATS/account limits but NO JetStream (the SYS account; the server refuses JS on it).
fn sys_limits() -> Value {
    json!({
        "subs": -1, "data": -1, "payload": -1, "imports": -1, "exports": -1,
        "wildcards": true, "conn": -1, "leaf": -1, "mem_storage": 0, "disk_storage": 0
    })
}

/// Generate a fresh operator → account(+signing key) → system-account chain for a space.
pub fn create_space_auth(space: &str) -> Result<SpaceAuth, AuthError> {
    let okp = KeyPair::new_operator();
    let akp = KeyPair::new_account();
    let askp = KeyPair::new_account(); // account signing key — what mints users
    let syskp = KeyPair::new_account();
    let sys_pub = syskp.public_key();

    let operator_jwt = jwt::encode_operator(&format!("parler-{}", token(space)), &okp, &sys_pub)?;
    let account_jwt = jwt::encode_account(
        &token(space),
        &akp.public_key(),
        &okp,
        &[askp.public_key()],
        data_limits(),
    )?;
    let sys_jwt = jwt::encode_account("SYS", &sys_pub, &okp, &[], sys_limits())?;

    let seed = |kp: &KeyPair| kp.seed().map_err(|e| AuthError::Nkeys(e.to_string()));
    Ok(SpaceAuth {
        space: space.to_string(),
        operator: OperatorMaterial {
            seed: seed(&okp)?,
            jwt: operator_jwt,
        },
        account: AccountMaterial {
            public: akp.public_key(),
            seed: seed(&akp)?,
            jwt: account_jwt,
            signing_seed: seed(&askp)?,
            signing_pub: askp.public_key(),
        },
        sys: SysMaterial {
            public: sys_pub,
            jwt: sys_jwt,
            signing_seed: Some(seed(&syskp)?),
        },
    })
}

/// Reduce a [`SpaceAuth`] to just the material a minting host needs (no operator/system seeds).
pub fn strip_space_auth(auth: &SpaceAuth) -> SpaceAuth {
    SpaceAuth {
        space: auth.space.clone(),
        operator: OperatorMaterial::default(),
        account: AccountMaterial {
            public: auth.account.public.clone(),
            signing_seed: auth.account.signing_seed.clone(),
            ..Default::default()
        },
        sys: SysMaterial::default(),
    }
}

pub struct ServerConfigOpts {
    pub port: u16,
    pub host: String,
    pub store_dir: String,
}

/// Render the `nats-server` config that trusts this space's operator and serves its accounts via the
/// in-config MEMORY resolver.
pub fn server_config(auth: &SpaceAuth, opts: &ServerConfigOpts) -> String {
    let store = serde_json::to_string(&opts.store_dir).unwrap_or_else(|_| "\".\"".into());
    format!(
        "# Generated by `parler up` — do not edit by hand.
host: {host}
port: {port}
max_control_line: 65536
jetstream {{ store_dir: {store} }}
operator: {op}
system_account: {sys}
resolver: MEMORY
resolver_preload: {{
  {acc_pub}: {acc_jwt}
  {sys_pub}: {sys_jwt}
}}
",
        host = opts.host,
        port = opts.port,
        store = store,
        op = auth.operator.jwt,
        sys = auth.sys.public,
        acc_pub = auth.account.public,
        acc_jwt = auth.account.jwt,
        sys_pub = auth.sys.public,
        sys_jwt = auth.sys.jwt,
    )
}

/// Options shaping a minted user's permissions.
#[derive(Debug, Clone, Default)]
pub struct MintOpts {
    /// Read ACL — channels an agent MAY read. Defaults to `["general"]`.
    pub allow_subscribe: Vec<String>,
    /// Post ACL — channels an agent may publish to. **Default-deny** (empty ⇒ no chat publish).
    pub allow_publish: Vec<String>,
    /// The agent's role — scopes its TASK-queue consumer to `svc_<role>`.
    pub role: Option<String>,
    /// Control service the agent may address. Defaults to `"manager"`.
    pub manager: Option<String>,
    /// Capabilities (e.g. `"spawn"`) — gate the privileged control-subject grant.
    pub capabilities: Vec<String>,
}

/// Mint a user creds file for an agent [`Identity`]: scope its permissions to `profile` and fold the
/// signed user JWT + the agent seed into a NATS creds file.
pub fn mint_creds(
    auth: &SpaceAuth,
    identity: &Identity,
    profile: Profile,
    opts: &MintOpts,
) -> Result<String, AuthError> {
    let signer = KeyPair::from_seed(&auth.account.signing_seed)
        .map_err(|e| AuthError::Nkeys(e.to_string()))?;
    let perms = permissions_for(profile, &auth.space, &identity.id, opts)?;
    let user_jwt = jwt::encode_user(
        profile.as_str(),
        &identity.id,
        &auth.account.public,
        &signer,
        perms,
    )?;
    Ok(fmt_creds(&user_jwt, &identity.seed))
}

/// Build the NATS user permission object for a profile — a default-deny allow-list. `manager` stays
/// permissive (the privileged provisioner host).
pub fn permissions_for(
    profile: Profile,
    space: &str,
    id: &str,
    opts: &MintOpts,
) -> Result<Value, AuthError> {
    match profile {
        Profile::Manager => Ok(json!({})), // privileged: allow-all defaults
        Profile::Delivery => Ok(delivery_permissions(space, id)),
        Profile::MembershipRw => Ok(membership_rw_permissions(space, id)),
        Profile::Observer => Ok(read_only_permissions(space, id, false)),
        Profile::Admin => Ok(read_only_permissions(space, id, true)),
        Profile::Agent => agent_permissions(space, id, opts),
    }
}

fn kv_stream(bucket: &str) -> String {
    format!("KV_{bucket}")
}

fn agent_permissions(space: &str, id: &str, opts: &MintOpts) -> Result<Value, AuthError> {
    let chat = chat_stream(space);
    let dm = dm_stream(space);
    let task = task_stream(space);
    let dlv = dlv_stream(space);
    let kv = kv_stream(&presence_bucket(space));
    let chkv = kv_stream(&channel_bucket(space));
    let dlvkv = kv_stream(&delivery_bucket(space));
    let inbox = format!("_INBOX_{id}.>");

    let allow_publish = &opts.allow_publish;
    let allow_subscribe = if opts.allow_subscribe.is_empty() {
        vec!["general".to_string()]
    } else {
        opts.allow_subscribe.clone()
    };
    for ch in allow_subscribe.iter().chain(allow_publish.iter()) {
        assert_valid_channel(ch)?;
    }
    let manager = opts.manager.clone().unwrap_or_else(|| CONTROL_PRIVILEGED.to_string());
    let chat_hist_d = chat_hist_durable(id);
    let dm_d = dm_durable(id);
    let dlv_d = dlv_durable(id);
    let svc_d = opts.role.as_ref().map(|r| task_durable(r));

    let mut pub_allow: Vec<String> = Vec::new();
    // peer publish — identity + channel scope. Default-deny: only declared allowPublish channels.
    for ch in allow_publish {
        pub_allow.push(chat_subject(space, id, ch)?);
    }
    pub_allow.push(unicast_subject(space, "*", id)); // DM any instance, as me
    pub_allow.push(anycast_subject(space, "*", id)); // anycast any role, as me
    pub_allow.push(control_service_subject(space, CONTROL_SELF_SERVICE, id)); // self stop/despawn
    pub_allow.push(control_service_subject(space, CONTROL_DELIVERY, id)); // durable join/leave/list
    pub_allow.push("$JS.API.INFO".into());
    pub_allow.push(format!("$JS.API.STREAM.INFO.{chat}"));
    pub_allow.push(format!("$JS.API.STREAM.INFO.{kv}"));
    pub_allow.push(format!("$JS.API.STREAM.INFO.{chkv}"));
    // CHAT history reads: one single-filter create grant per allowSubscribe channel (the server pins
    // the trailing filter token to the body), bounding history to the read ACL.
    for ch in &allow_subscribe {
        pub_allow.push(format!(
            "$JS.API.CONSUMER.CREATE.{chat}.{chat_hist_d}.{}",
            chat_subject(space, "*", ch)?
        ));
    }
    pub_allow.push(format!("$JS.API.CONSUMER.INFO.{chat}.{chat_hist_d}"));
    pub_allow.push(format!("$JS.API.CONSUMER.MSG.NEXT.{chat}.{chat_hist_d}"));
    pub_allow.push(format!("$JS.API.CONSUMER.DELETE.{chat}.{chat_hist_d}"));
    // DM consumer: BIND ONLY its own pre-created durable.
    pub_allow.push(format!("$JS.API.CONSUMER.INFO.{dm}.{dm_d}"));
    pub_allow.push(format!("$JS.API.CONSUMER.MSG.NEXT.{dm}.{dm_d}"));
    pub_allow.push(format!("$JS.ACK.{dm}.{dm_d}.>"));
    // Plane-3 DELIVER consumer: BIND ONLY its own pre-created dlv_<id>.
    pub_allow.push(format!("$JS.API.CONSUMER.INFO.{dlv}.{dlv_d}"));
    pub_allow.push(format!("$JS.API.CONSUMER.MSG.NEXT.{dlv}.{dlv_d}"));
    pub_allow.push(format!("$JS.ACK.{dlv}.{dlv_d}.>"));
    // Presence: watch (public roster) + flow control + PUT OWN KEY ONLY.
    pub_allow.push(format!("$JS.API.CONSUMER.CREATE.{kv}.>"));
    pub_allow.push(format!("$JS.API.CONSUMER.INFO.{kv}.>"));
    pub_allow.push("$JS.FC.>".into());
    pub_allow.push(format!("$KV.{}.{id}", presence_bucket(space))); // own presence key only
    // Channel registry: read-only (watch + direct kv.get).
    pub_allow.push(format!("$JS.API.STREAM.MSG.GET.{chkv}"));
    pub_allow.push(format!("$JS.API.CONSUMER.CREATE.{chkv}.>"));
    pub_allow.push(format!("$JS.API.CONSUMER.INFO.{chkv}.>"));
    // Delivery lease/readiness: READ-ONLY (non-gating delivery-health surface).
    pub_allow.push(format!("$JS.API.STREAM.INFO.{dlvkv}"));
    pub_allow.push(format!("$JS.API.STREAM.MSG.GET.{dlvkv}"));
    if let Some(svc_d) = &svc_d {
        // TASK consumer: BIND ONLY its own role's pre-created durable.
        pub_allow.push(format!("$JS.API.CONSUMER.INFO.{task}.{svc_d}"));
        pub_allow.push(format!("$JS.API.CONSUMER.MSG.NEXT.{task}.{svc_d}"));
        pub_allow.push(format!("$JS.ACK.{task}.{svc_d}.>"));
    }
    if opts.capabilities.iter().any(|c| c == "spawn") {
        // Spawn capability → the PRIVILEGED control subject (default-deny otherwise).
        pub_allow.push(control_service_subject(space, &manager, id));
    }
    // Explicit create-deny on the two streams whose create-time filter is the attack surface.
    let pub_deny = vec![
        format!("$JS.API.CONSUMER.CREATE.{dm}"),
        format!("$JS.API.CONSUMER.CREATE.{dm}.>"),
        format!("$JS.API.CONSUMER.DURABLE.CREATE.{dm}.>"),
        format!("$JS.API.CONSUMER.CREATE.{task}"),
        format!("$JS.API.CONSUMER.CREATE.{task}.>"),
        format!("$JS.API.CONSUMER.DURABLE.CREATE.{task}.>"),
        format!("$JS.API.CONSUMER.CREATE.{dlv}"),
        format!("$JS.API.CONSUMER.CREATE.{dlv}.>"),
        format!("$JS.API.CONSUMER.DURABLE.CREATE.{dlv}.>"),
    ];
    // CHAT live read boundary: native sub.allow over chat.*.<channel>, one per allowSubscribe.
    let mut sub_allow = vec![
        inbox,
        format!("{}.>", control_service_subject(space, CONTROL_DELIVERY, id)),
    ];
    for ch in &allow_subscribe {
        sub_allow.push(chat_subject(space, "*", ch)?);
    }
    Ok(json!({
        "pub": { "allow": pub_allow, "deny": pub_deny },
        "sub": { "allow": sub_allow }
    }))
}

fn read_only_permissions(space: &str, id: &str, admin: bool) -> Value {
    let chat = chat_stream(space);
    let dm = dm_stream(space);
    let kv = kv_stream(&presence_bucket(space));
    let chkv = kv_stream(&channel_bucket(space));
    let memkv = kv_stream(&membership_bucket(space));
    let inbox = format!("_INBOX_{id}.>");

    let sub = if admin {
        vec![format!("{}.>", space_prefix(space)), inbox.clone()]
    } else {
        vec![chat_wildcard(space), inbox.clone()]
    };
    let mut allow = vec![
        "$JS.API.INFO".to_string(),
        format!("$JS.API.STREAM.INFO.{chat}"),
        format!("$JS.API.STREAM.INFO.{kv}"),
        format!("$JS.API.CONSUMER.CREATE.{chat}"),
        format!("$JS.API.CONSUMER.CREATE.{chat}.>"),
        format!("$JS.API.CONSUMER.INFO.{chat}.>"),
        format!("$JS.API.CONSUMER.MSG.NEXT.{chat}.>"),
        format!("$JS.API.CONSUMER.DELETE.{chat}.>"),
        format!("$JS.ACK.{chat}.>"),
        format!("$JS.API.CONSUMER.CREATE.{kv}.>"),
        format!("$JS.API.CONSUMER.INFO.{kv}.>"),
        format!("$JS.API.STREAM.INFO.{chkv}"),
        format!("$JS.API.STREAM.MSG.GET.{chkv}"),
        format!("$JS.API.CONSUMER.CREATE.{chkv}.>"),
        format!("$JS.API.CONSUMER.INFO.{chkv}.>"),
        format!("$JS.API.CONSUMER.DELETE.{chkv}.>"),
        format!("$JS.API.STREAM.INFO.{memkv}"),
        format!("$JS.API.STREAM.MSG.GET.{memkv}"),
        format!("$JS.API.CONSUMER.CREATE.{memkv}.>"),
        format!("$JS.API.CONSUMER.INFO.{memkv}.>"),
        format!("$JS.API.CONSUMER.DELETE.{memkv}.>"),
        "$JS.FC.>".to_string(),
    ];
    if admin {
        allow.extend([
            format!("$JS.API.STREAM.INFO.{dm}"),
            format!("$JS.API.CONSUMER.CREATE.{dm}"),
            format!("$JS.API.CONSUMER.CREATE.{dm}.>"),
            format!("$JS.API.CONSUMER.INFO.{dm}.>"),
            format!("$JS.API.CONSUMER.MSG.NEXT.{dm}.>"),
            format!("$JS.API.CONSUMER.DELETE.{dm}.>"),
            format!("$JS.ACK.{dm}.>"),
        ]);
    }
    json!({ "sub": { "allow": sub }, "pub": { "allow": allow } })
}

/// The scoped `delivery` daemon permission set (server-side Plane-3 infra; least-privilege).
fn delivery_permissions(space: &str, id: &str) -> Value {
    let p = space_prefix(space);
    let chat = chat_stream(space);
    let inbox_s = inbox_stream(space);
    let dlv = dlv_stream(space);
    let pkv = kv_stream(&presence_bucket(space));
    let chkv = kv_stream(&channel_bucket(space));
    let mkv = kv_stream(&members_bucket(space));
    let akv = kv_stream(&acl_bucket(space));
    let dkv = kv_stream(&delivery_bucket(space));
    let kv_read = |bucket: &str| -> Vec<String> {
        vec![
            format!("$JS.API.STREAM.INFO.{bucket}"),
            format!("$JS.API.STREAM.MSG.GET.{bucket}"),
            format!("$JS.API.CONSUMER.CREATE.{bucket}.>"),
            format!("$JS.API.CONSUMER.INFO.{bucket}.>"),
            format!("$JS.API.CONSUMER.DELETE.{bucket}.>"),
        ]
    };
    let mut pub_allow = vec![
        "$JS.API.INFO".to_string(),
        format!("$JS.API.STREAM.INFO.{chat}"),
        format!("$JS.API.STREAM.INFO.{inbox_s}"),
        format!("$JS.API.STREAM.INFO.{dlv}"),
        format!("$JS.API.CONSUMER.CREATE.{chat}.>"),
        format!("$JS.API.CONSUMER.DURABLE.CREATE.{chat}.>"),
        format!("$JS.API.CONSUMER.INFO.{chat}.>"),
        format!("$JS.API.CONSUMER.MSG.NEXT.{chat}.>"),
        format!("$JS.API.CONSUMER.DELETE.{chat}.>"),
        format!("$JS.ACK.{chat}.>"),
        format!("$JS.API.CONSUMER.CREATE.{inbox_s}.{INBOX_READER_DURABLE}.>"),
        format!("$JS.API.CONSUMER.DURABLE.CREATE.{inbox_s}.{INBOX_READER_DURABLE}"),
        format!("$JS.API.CONSUMER.INFO.{inbox_s}.{INBOX_READER_DURABLE}"),
        format!("$JS.API.CONSUMER.MSG.NEXT.{inbox_s}.{INBOX_READER_DURABLE}"),
        format!("$JS.API.CONSUMER.DELETE.{inbox_s}.{INBOX_READER_DURABLE}"),
        format!("$JS.ACK.{inbox_s}.{INBOX_READER_DURABLE}.>"),
        "$JS.FC.>".to_string(),
    ];
    pub_allow.extend(kv_read(&pkv));
    pub_allow.extend(kv_read(&chkv));
    pub_allow.extend(kv_read(&mkv));
    pub_allow.extend(kv_read(&akv));
    pub_allow.push(format!("$KV.{}.>", members_bucket(space))); // members-KV write
    pub_allow.push(format!("$JS.API.STREAM.INFO.{dkv}"));
    pub_allow.push(format!("$JS.API.STREAM.MSG.GET.{dkv}"));
    pub_allow.push(format!("$KV.{}.lease.*", delivery_bucket(space))); // lease keys only
    pub_allow.push(format!("{p}.dinbox.*")); // fan-out target
    pub_allow.push(format!("{p}.dlv.*")); // post-auth handoff
    pub_allow.push(format!("{p}.ctl.delivery.*.reply.>")); // control replies only

    let sub_allow = vec![
        format!("_INBOX_{id}.>"),
        format!("{p}.ctl.delivery.*"), // serve the delivery control service
    ];
    json!({ "pub": { "allow": pub_allow }, "sub": { "allow": sub_allow } })
}

/// The scoped DATA-account `membership-rw` permission set (graph feed writer; least-privilege).
fn membership_rw_permissions(space: &str, id: &str) -> Value {
    let mkv = kv_stream(&members_bucket(space));
    let memkv = kv_stream(&membership_bucket(space));
    let kv_read = |bucket: &str| -> Vec<String> {
        vec![
            format!("$JS.API.STREAM.INFO.{bucket}"),
            format!("$JS.API.STREAM.MSG.GET.{bucket}"),
            format!("$JS.API.CONSUMER.CREATE.{bucket}.>"),
            format!("$JS.API.CONSUMER.INFO.{bucket}.>"),
            format!("$JS.API.CONSUMER.DELETE.{bucket}.>"),
        ]
    };
    let mut pub_allow = vec!["$JS.API.INFO".to_string()];
    pub_allow.extend(kv_read(&mkv));
    pub_allow.extend(kv_read(&memkv));
    pub_allow.push(format!("$KV.{}.>", membership_bucket(space))); // write derived feed
    pub_allow.push("$JS.FC.>".to_string());
    json!({ "pub": { "allow": pub_allow }, "sub": { "allow": [format!("_INBOX_{id}.>")] } })
}

/// The scoped SYSTEM-account `membership-observer` permission set (graph feed CONNZ reader). An
/// explicit block is mandatory: a system-account user with no permissions defaults to allow-all.
pub fn membership_observer_permissions(account_id: &str) -> Value {
    json!({
        "pub": { "allow": [connz_request_subject(account_id)] },
        "sub": { "allow": [
            format!("{MEMBERSHIP_INBOX_PREFIX}.>"),
            account_connect_subject(account_id),
            account_disconnect_subject(account_id),
        ]}
    })
}

/// Format a NATS user creds file (JWT + seed blocks), matching the standard nsc/`nats` layout.
pub fn fmt_creds(user_jwt: &str, user_seed: &str) -> String {
    format!(
        "-----BEGIN NATS USER JWT-----
{user_jwt}
------END NATS USER JWT------

************************* IMPORTANT *************************
NKEY Seed printed below can be used to connect.

Do NOT share this with anyone, especially this Seed.

-----BEGIN USER NKEY SEED-----
{user_seed}
------END USER NKEY SEED------

*************************************************************
"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::new_identity;

    #[test]
    fn space_auth_chain_has_distinct_keys() {
        let auth = create_space_auth("main").unwrap();
        assert!(auth.operator.seed.starts_with("SO"));
        assert!(auth.account.public.starts_with('A'));
        assert!(auth.account.signing_pub.starts_with('A'));
        assert_ne!(auth.account.public, auth.account.signing_pub);
        assert!(auth.sys.public.starts_with('A'));
        assert_ne!(auth.account.public, auth.sys.public);
        assert!(auth.sys.signing_seed.is_some());
    }

    #[test]
    fn strip_keeps_only_minting_material() {
        let auth = create_space_auth("main").unwrap();
        let stripped = strip_space_auth(&auth);
        assert_eq!(stripped.account.public, auth.account.public);
        assert_eq!(stripped.account.signing_seed, auth.account.signing_seed);
        assert!(stripped.operator.seed.is_empty());
        assert!(stripped.account.seed.is_empty());
        assert!(stripped.sys.public.is_empty());
    }

    #[test]
    fn agent_acl_is_default_deny_publish_and_scopes_reads() {
        let id = new_identity().unwrap();
        let perms = permissions_for(
            Profile::Agent,
            "main",
            &id.id,
            &MintOpts {
                allow_publish: vec!["general".into()],
                allow_subscribe: vec!["general".into(), "team.>".into()],
                ..Default::default()
            },
        )
        .unwrap();
        let pub_allow = perms["pub"]["allow"].as_array().unwrap();
        let has = |s: &str| pub_allow.iter().any(|v| v == s);
        // Declared channel ⇒ a publish grant; an undeclared one ⇒ none.
        assert!(has(&chat_subject("main", &id.id, "general").unwrap()));
        assert!(!has(&chat_subject("main", &id.id, "secret").unwrap()));
        // Reads scoped to the ACL via native sub.allow.
        let sub_allow = perms["sub"]["allow"].as_array().unwrap();
        assert!(sub_allow.iter().any(|v| v == &chat_subject("main", "*", "general").unwrap()));
        assert!(sub_allow.iter().any(|v| v == &chat_subject("main", "*", "team.>").unwrap()));
        // No spawn capability ⇒ no privileged manager control subject.
        assert!(!has(&control_service_subject("main", CONTROL_PRIVILEGED, &id.id)));
    }

    #[test]
    fn spawn_capability_grants_manager_control() {
        let id = new_identity().unwrap();
        let perms = permissions_for(
            Profile::Agent,
            "main",
            &id.id,
            &MintOpts {
                capabilities: vec!["spawn".into()],
                ..Default::default()
            },
        )
        .unwrap();
        let pub_allow = perms["pub"]["allow"].as_array().unwrap();
        assert!(pub_allow
            .iter()
            .any(|v| v == &control_service_subject("main", CONTROL_PRIVILEGED, &id.id)));
    }

    #[test]
    fn creds_round_trip_through_id_from_creds() {
        let auth = create_space_auth("main").unwrap();
        let id = new_identity().unwrap();
        let creds = mint_creds(&auth, &id, Profile::Agent, &MintOpts::default()).unwrap();
        assert_eq!(crate::identity::id_from_creds(&creds).unwrap(), id.id);
    }

    #[test]
    fn manager_is_allow_all() {
        let id = new_identity().unwrap();
        let perms = permissions_for(Profile::Manager, "main", &id.id, &MintOpts::default()).unwrap();
        assert_eq!(perms, json!({}));
    }
}
