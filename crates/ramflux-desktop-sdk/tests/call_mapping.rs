// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Every supported contract method maps to its daemon request, and every
//! unsupported one is refused with its reason. Routes and group links come
//! from the directories the fixture's answers produce.

mod common;

use ramflux_desktop_core::{
    AccountId, ContextId, ContextKind, Effect, FailureCode, SecretInput, methods,
};
use ramflux_desktop_sdk::capabilities;
use ramflux_desktop_sdk::directory::Directory;
use ramflux_desktop_sdk::plan::fetch::{Fetch, Paged};
use ramflux_desktop_sdk::plan::{Known, Plan, plan};
use serde_json::{Value, json};

struct Fake(Directory);

impl Known for Fake {
    fn context_kind(&self, context: &ContextId) -> Option<ContextKind> {
        match context.as_str() {
            "thread-team" => Some(ContextKind::Group),
            "dm-ana" | "dm-unrouted" => Some(ContextKind::Direct),
            _ => None,
        }
    }
    fn directory(&self) -> Option<&Directory> {
        Some(&self.0)
    }
    fn self_device(&self) -> Option<&str> {
        Some("device-me")
    }
    fn daemon_grant_id(&self, approval: &str) -> Option<&str> {
        (approval == "appr-1").then_some("grant-1")
    }
}

fn fake() -> Fake {
    Fake(common::directory())
}

fn call(method: &str, body: Value) -> Effect {
    Effect::Call { method: method.to_owned(), body }
}

fn secret(method: &str, body: Value, secrets: &[&str]) -> Effect {
    Effect::SecretCall {
        method: method.to_owned(),
        body,
        secrets: secrets.iter().map(|s| SecretInput::new(*s)).collect(),
    }
}

/// (daemon method, account, wire body, refresh reads).
fn mapped(effect: Effect) -> (String, Option<String>, Value, Vec<Fetch>) {
    let account = AccountId::new("acct-main");
    match plan(Some(&account), effect, &fake()).plan {
        Plan::Call { request, refresh } => {
            let wire = request.wire_body();
            (request.method.clone(), request.account.clone(), wire, refresh)
        }
        other => panic!("expected a call, got {other:?}"),
    }
}

#[test]
fn entry_and_node_methods_map_to_daemon_requests() {
    let (m, a, b, _) = mapped(call(methods::NODE_ADD, json!({ "url": "https://n.example" })));
    assert_eq!((m.as_str(), a, b), ("node.add", None, json!({ "url": "https://n.example" })));
    let (m, _, b, _) =
        mapped(call(methods::NODE_CONFIRM, json!({ "node_id": "n", "pin_source": "Manual" })));
    assert_eq!((m.as_str(), b), ("node.confirm", json!({ "node_id": "n", "pin_source": "tofu" })));
    let (_, _, b, _) =
        mapped(call(methods::NODE_CONFIRM, json!({ "node_id": "n", "pin_source": "Invite" })));
    assert_eq!(b["pin_source"], "invitation");
    for (method, daemon) in [
        (methods::NODE_PROBE, "node.probe"),
        (methods::NODE_REMOVE, "node.remove"),
        (methods::NODE_SET_DEFAULT, "node.set_default"),
    ] {
        let (m, _, b, _) = mapped(call(method, json!({ "node_id": "n" })));
        assert_eq!((m.as_str(), b), (daemon, json!({ "node_id": "n" })));
    }
    let (m, _, b, _) =
        mapped(call(methods::NODE_SET_ENABLED, json!({ "node_id": "n", "enabled": false })));
    assert_eq!((m.as_str(), b), ("node.set_enabled", json!({ "node_id": "n", "enabled": false })));
    let (m, a, b, _) =
        mapped(secret(methods::ACCOUNT_UNLOCK, json!({ "account_id": "acct-x" }), &["pw"]));
    assert_eq!(
        (m.as_str(), a.as_deref(), b),
        ("account.unlock", Some("acct-x"), json!({ "passphrase": "pw" }))
    );
    let (m, a, b, _) = mapped(secret(
        methods::ACCOUNT_CREATE,
        json!({ "kind": "HeadlessAi", "label": "Bot", "node_id": "n-harbor" }),
        &["pw"],
    ));
    assert_eq!((m.as_str(), a), ("account.create", None));
    assert_eq!(b["client_mode"], "headless_ai");
    assert_eq!(b["node_id"], "n-harbor");
    assert_eq!(b["account_secret"], "pw");
    assert_eq!(b["device_seed"].as_array().map(Vec::len), Some(32));
    let (m, _, b, _) =
        mapped(secret(methods::ACCOUNT_BACKUP_IMPORT, json!({ "path": "/b" }), &["pw"]));
    assert_eq!(
        (m.as_str(), b),
        ("account.backup.import", json!({ "input_path": "/b", "passphrase": "pw" }))
    );
    let (m, a, b, _) = mapped(secret(
        methods::ACCOUNT_BACKUP_EXPORT,
        json!({ "account_id": "acct-main", "path": "/out.backup" }),
        &["pw"],
    ));
    assert_eq!(
        (m.as_str(), a.as_deref(), b),
        (
            "account.backup.export",
            Some("acct-main"),
            json!({ "output_path": "/out.backup", "passphrase": "pw" })
        )
    );
    let (m, a, _, _) = mapped(call(methods::ACCOUNT_LOCK, json!({ "account_id": "acct-main" })));
    assert_eq!((m.as_str(), a.as_deref()), ("account.lock", Some("acct-main")));
    let (m, _, b, _) = mapped(secret(
        methods::ACCOUNT_ROTATE_PASSPHRASE,
        json!({ "account_id": "acct-main" }),
        &["old", "new"],
    ));
    assert_eq!(
        (m.as_str(), b),
        ("account.passphrase.rotate", json!({ "old_passphrase": "old", "new_passphrase": "new" }))
    );
}

#[test]
fn account_methods_map_to_daemon_requests() {
    let (m, a, b, r) =
        mapped(call(methods::GRANT_APPROVE, json!({ "grant_id": "appr-1", "args_hash": "h" })));
    assert_eq!(
        (m.as_str(), a.as_deref(), b, r),
        (
            "grant.approve",
            Some("acct-main"),
            json!({ "approval_id": "appr-1" }),
            vec![Fetch::Grants]
        )
    );
    let (m, _, b, _) = mapped(call(methods::GRANT_DENY, json!({ "grant_id": "appr-2" })));
    assert_eq!((m.as_str(), b), ("grant.deny", json!({ "approval_id": "appr-2" })));
    let (m, _, b, _) = mapped(call(methods::GRANT_REVOKE, json!({ "grant_id": "appr-1" })));
    assert_eq!(
        (m.as_str(), b),
        ("grant.revoke", json!({ "grant_id": "grant-1" })),
        "the daemon's grant id"
    );
    let (m, _, b, r) = mapped(call(methods::STANDING_REVOKE, json!({ "approval_id": "sa-1" })));
    assert_eq!(
        (m.as_str(), b, r),
        (
            "grant.revoke_standing_approval",
            json!({ "standing_approval_id": "sa-1" }),
            vec![Fetch::StandingApprovals]
        )
    );
    let (m, _, b, _) =
        mapped(call(methods::CONTEXT_MUTE, json!({ "context_id": "dm-ana", "muted": true })));
    assert_eq!(
        (m.as_str(), b),
        ("conversation.mute", json!({ "conversation_id": "dm-ana", "unmute": false }))
    );
    let (m, _, b, _) = mapped(call(
        methods::CONTEXT_DISAPPEARING,
        json!({ "context_id": "dm-ana", "seconds": 3600 }),
    ));
    assert_eq!(
        (m.as_str(), b),
        ("conversation.disappearing.set", json!({ "conversation_id": "dm-ana", "ttl_secs": 3600 }))
    );
    let (m, _, b, r) = mapped(call(methods::DEVICE_REVOKE, json!({ "device_id": "device-desk" })));
    assert_eq!(
        (m.as_str(), b, r),
        ("device.revoke", json!({ "device_id": "device-desk" }), vec![Fetch::DeviceList])
    );
    let (m, _, b, r) =
        mapped(call(methods::SEND_MESSAGE, json!({ "context_id": "thread-team", "text": "hi" })));
    assert_eq!(m, "group.send");
    assert_eq!(
        (b["group_id"].clone(), b["conversation_id"].clone(), b["sender_id"].clone()),
        (json!("g-team"), json!("thread-team"), json!("device-me"))
    );
    assert_eq!(b["plaintext_body_base64"], "aGk");
    assert_eq!(
        r,
        vec![Fetch::GroupReceive {
            context: ContextId::new("thread-team"),
            group: "g-team".to_owned()
        }]
    );
    let (m, _, b, r) =
        mapped(call(methods::SEND_MESSAGE, json!({ "context_id": "dm-ana", "text": "hi" })));
    assert_eq!(
        (m.as_str(), b["target_delivery_id"].clone()),
        ("message.submit", json!("target-ana"))
    );
    assert_eq!(
        r,
        vec![Fetch::Messages(ContextId::new("dm-ana")), Fetch::first(Paged::Conversations)]
    );
    for (method, daemon) in [
        (methods::GROUP_KICK, "group.member.kick"),
        (methods::GROUP_BAN, "group.member.ban"),
        (methods::GROUP_REMOVE_MEMBER, "group.member.remove"),
    ] {
        let (m, _, b, _) = mapped(call(
            method,
            json!({ "context_id": "thread-team", "participant_id": "principal-bo" }),
        ));
        assert_eq!(
            (m.as_str(), b),
            (
                daemon,
                json!({ "group_id": "g-team", "actor_id": "device-me", "member_id": "device-bo" })
            )
        );
    }
    let (m, _, b, _) = mapped(call(
        methods::GROUP_SET_ROLE,
        json!({ "context_id": "thread-team", "participant_id": "principal-bo", "role": "Admin" }),
    ));
    assert_eq!((m.as_str(), b["role"].clone()), ("group.role.set", json!("admin")));
    let (_, _, b, _) = mapped(call(
        methods::GROUP_SET_ROLE,
        json!({ "context_id": "thread-team", "participant_id": "principal-bo", "role": "Bot" }),
    ));
    assert_eq!(b["role"], "bot");
}

#[test]
fn local_and_unsupported_methods_never_reach_the_daemon() {
    let account = AccountId::new("acct-main");
    assert!(matches!(
        plan(
            Some(&account),
            call(methods::CONTEXT_PIN, json!({ "context_id": "dm-ana" })),
            &fake()
        )
        .plan,
        Plan::Local
    ));
    assert!(matches!(
        plan(None, call(methods::DIAGNOSTICS_EXPORT, json!({ "path": "/tmp/d" })), &fake()).plan,
        Plan::Diagnostics { .. }
    ));
    for method in capabilities::unsupported().keys().filter(|m| !m.contains('@')) {
        let planned = plan(Some(&account), call(method, json!({})), &fake());
        assert!(
            matches!(&planned.plan, Plan::Refuse { code: FailureCode::UnsupportedByBackend, message } if message.starts_with("backend.gap.")),
            "{method}: {:?}",
            planned.plan
        );
    }
    let unrouted = plan(
        Some(&account),
        call(methods::SEND_MESSAGE, json!({ "context_id": "dm-unrouted", "text": "x" })),
        &fake(),
    );
    assert!(matches!(
        unrouted.plan,
        Plan::Refuse { code: FailureCode::UnsupportedByBackend, ref message } if message == "backend.gap.no_route"
    ));
    let unknown_role = plan(
        Some(&account),
        call(
            methods::GROUP_SET_ROLE,
            json!({ "context_id": "thread-team", "participant_id": "principal-bo", "role": "Guest" }),
        ),
        &fake(),
    );
    assert!(
        matches!(unknown_role.plan, Plan::Refuse { code: FailureCode::Daemon(_), .. }),
        "the desktop has exactly the daemon's roles"
    );
    let no_account = plan(None, call(methods::GRANT_APPROVE, json!({ "grant_id": "a" })), &fake());
    assert!(matches!(no_account.plan, Plan::Refuse { .. }));
}
