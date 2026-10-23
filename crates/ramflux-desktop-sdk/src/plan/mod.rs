// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Effects to daemon requests.
//!
//! Desktop contract methods are close to the daemon's but not identical:
//! names differ (`message.send` is `group.send` for a group), ids differ
//! (a desktop grant id is the daemon's approval id) and bodies differ. Each
//! supported method maps to exactly one request plus the reads that refresh
//! what it changed; unsupported methods are refused with their reason.
//! Every request that changes daemon state gets a fresh retry key here, once
//! per effect (see [`crate::bus::idempotency`]).
//!
//! - [`fetch`]: the reads.
//! - [`identity`]: fresh identity material for account creation.
//! - [`contact`]: contact effects and the links they name.

pub mod contact;
pub mod fetch;
pub mod identity;

use ramflux_desktop_core::{
    AccountId, AccountKind, ContextId, ContextKind, Effect, FailureCode, NodeId, PinSource, Role,
    SecretInput, methods,
};
use serde_json::{Value, json};

use crate::bus::{BusRequest, idempotency};
use crate::capabilities;
pub use contact::KnownLink;
use fetch::Fetch;

/// What the source learned that a request needs.
pub trait Known {
    /// Kind of a known context.
    fn context_kind(&self, context: &ContextId) -> Option<ContextKind>;
    /// This device's id in the account (the sender and group actor id).
    fn self_device(&self) -> Option<&str>;
    /// The daemon's grant id for an approved desktop grant (approval id).
    fn daemon_grant_id(&self, approval: &str) -> Option<&str>;
    /// This account's principal id (the requester of the links it adds).
    fn self_principal(&self) -> Option<&str> {
        None
    }
    /// The contact link with `participant`, as the daemon last listed it.
    fn contact_link(&self, participant: &str) -> Option<&KnownLink> {
        let _ = participant;
        None
    }
}

/// How to perform one effect.
#[derive(Debug)]
pub enum Plan {
    /// One request, then the reads that refresh what it changed.
    Call {
        /// The request.
        request: BusRequest,
        /// Reads after success.
        refresh: Vec<Fetch>,
    },
    /// A desktop-local preference; the translator echoes it.
    Local,
    /// Write the redacted diagnostics bundle to `path`.
    Diagnostics {
        /// Destination chosen by the user.
        path: String,
    },
    /// The effect cannot be performed.
    Refuse {
        /// Classification.
        code: FailureCode,
        /// Message or reason key.
        message: String,
    },
}

/// One planned effect.
#[derive(Debug)]
pub struct Planned {
    /// Contract method.
    pub method: String,
    /// The effect's (secret-free) body.
    pub body: Value,
    /// How to perform it.
    pub plan: Plan,
}

/// Plans `effect` for the scope's `account` (`None` for the daemon scope).
pub fn plan(account: Option<&AccountId>, effect: Effect, known: &impl Known) -> Planned {
    let (method, body, secrets) = match effect {
        Effect::Call { method, body } => (method, body, Vec::new()),
        Effect::SecretCall { method, body, secrets } => (method, body, secrets),
        Effect::RequireConfirmation { .. } => {
            return Planned {
                method: String::new(),
                body: Value::Null,
                plan: refuse_invalid("a confirmation request is not an effect"),
            };
        }
    };
    let plan = if let Some(reason) = capabilities::reason_for(&method) {
        Plan::Refuse { code: FailureCode::UnsupportedByBackend, message: reason.to_owned() }
    } else {
        keyed(map(account, &method, &body, secrets, known))
    };
    Planned { method, body, plan }
}

/// Gives a mutating request its retry key.
fn keyed(plan: Plan) -> Plan {
    match plan {
        Plan::Call { request, refresh } if idempotency::is_mutation(&request.method) => {
            match idempotency::fresh_key() {
                Ok(key) => Plan::Call { request: request.with_operation_id(key), refresh },
                Err(message) => refuse_invalid(&message),
            }
        }
        other => other,
    }
}

fn refuse_invalid(message: &str) -> Plan {
    Plan::Refuse { code: FailureCode::Daemon("invalid".to_owned()), message: message.to_owned() }
}

fn text<'a>(body: &'a Value, field: &str) -> &'a str {
    body.get(field).and_then(Value::as_str).unwrap_or_default()
}

fn call(request: BusRequest) -> Plan {
    Plan::Call { request, refresh: Vec::new() }
}

fn map(
    account: Option<&AccountId>,
    method: &str,
    body: &Value,
    mut secrets: Vec<SecretInput>,
    known: &impl Known,
) -> Plan {
    let acct = account.map(AccountId::as_str);
    let node = || json!({ "node_id": text(body, "node_id") });
    match method {
        methods::NODE_ADD => {
            call(BusRequest::new(None, "node", "node.add", json!({ "url": text(body, "url") })))
        }
        methods::NODE_CONFIRM => {
            let pin = serde_json::from_value::<PinSource>(body["pin_source"].clone()).ok();
            let pin = match pin {
                Some(PinSource::Manual) => "tofu",
                Some(PinSource::Invite) => "invitation",
                Some(PinSource::Bootstrap) => "builtin",
                None => return refuse_invalid("unknown pin source"),
            };
            let request = json!({ "node_id": text(body, "node_id"), "pin_source": pin });
            call(BusRequest::new(None, "node", "node.confirm", request))
        }
        methods::NODE_PROBE => call(BusRequest::new(None, "node", "node.probe", node())),
        methods::NODE_REMOVE => call(BusRequest::new(None, "node", "node.remove", node())),
        methods::NODE_SET_DEFAULT => {
            call(BusRequest::new(None, "node", "node.set_default", node()))
        }
        methods::NODE_SET_ENABLED => {
            let request = json!({ "node_id": text(body, "node_id"), "enabled": body["enabled"] });
            call(BusRequest::new(None, "node", "node.set_enabled", request))
        }
        methods::ACCOUNT_UNLOCK => match secrets.pop() {
            Some(secret) => call(
                BusRequest::new(
                    Some(text(body, "account_id")),
                    "account",
                    "account.unlock",
                    json!({}),
                )
                .with_secret("passphrase", secret),
            ),
            None => refuse_invalid("missing passphrase"),
        },
        methods::ACCOUNT_CREATE => {
            let kind =
                serde_json::from_value::<AccountKind>(body["kind"].clone()).unwrap_or_default();
            let Some(secret) = secrets.pop() else { return refuse_invalid("missing passphrase") };
            match identity::create_request(kind, &NodeId::new(text(body, "node_id")), secret) {
                Ok((_, request)) => call(request),
                Err(message) => refuse_invalid(&message),
            }
        }
        methods::ACCOUNT_BACKUP_IMPORT => match secrets.pop() {
            Some(secret) => call(
                BusRequest::new(
                    None,
                    "account",
                    "account.backup.import",
                    json!({ "input_path": text(body, "path") }),
                )
                .with_secret("passphrase", secret),
            ),
            None => refuse_invalid("missing passphrase"),
        },
        methods::ACCOUNT_BACKUP_EXPORT => match secrets.pop() {
            Some(secret) => call(
                BusRequest::new(
                    Some(text(body, "account_id")),
                    "account",
                    "account.backup.export",
                    json!({ "output_path": text(body, "path") }),
                )
                .with_secret("passphrase", secret),
            ),
            None => refuse_invalid("missing backup passphrase"),
        },
        methods::ACCOUNT_LOCK => call(BusRequest::new(
            Some(text(body, "account_id")),
            "account",
            "account.lock",
            json!({}),
        )),
        methods::ACCOUNT_ROTATE_PASSPHRASE => {
            let mut secrets = secrets.into_iter();
            match (secrets.next(), secrets.next()) {
                (Some(current), Some(next)) => call(
                    BusRequest::new(
                        Some(text(body, "account_id")),
                        "account",
                        "account.passphrase.rotate",
                        json!({}),
                    )
                    .with_secret("old_passphrase", current)
                    .with_secret("new_passphrase", next),
                ),
                _ => refuse_invalid("missing passphrases"),
            }
        }
        methods::DIAGNOSTICS_EXPORT => Plan::Diagnostics { path: text(body, "path").to_owned() },
        methods::CONTEXT_PIN | methods::CONTEXT_UNPIN => Plan::Local,
        _ if acct.is_none() => refuse_invalid("this action needs an unlocked account"),
        _ => account_call(acct, method, body, known),
    }
}

fn account_call(acct: Option<&str>, method: &str, body: &Value, known: &impl Known) -> Plan {
    let context = text(body, "context_id");
    match method {
        methods::GRANT_APPROVE | methods::GRANT_DENY => {
            let daemon =
                if method == methods::GRANT_APPROVE { "grant.approve" } else { "grant.deny" };
            let request = json!({ "approval_id": text(body, "grant_id") });
            Plan::Call {
                request: BusRequest::new(acct, "grant", daemon, request),
                refresh: vec![Fetch::Grants],
            }
        }
        methods::GRANT_REVOKE => {
            let approval = text(body, "grant_id");
            let grant = known.daemon_grant_id(approval).unwrap_or(approval);
            call(BusRequest::new(acct, "grant", "grant.revoke", json!({ "grant_id": grant })))
        }
        methods::STANDING_REVOKE => Plan::Call {
            request: BusRequest::new(
                acct,
                "grant",
                "grant.revoke_standing_approval",
                json!({ "standing_approval_id": text(body, "approval_id") }),
            ),
            refresh: vec![Fetch::StandingApprovals],
        },
        methods::CONTEXT_MUTE => {
            let muted = body["muted"].as_bool().unwrap_or(false);
            call(BusRequest::new(
                acct,
                "conversation",
                "conversation.mute",
                json!({ "conversation_id": context, "unmute": !muted }),
            ))
        }
        methods::CONTEXT_DISAPPEARING => call(BusRequest::new(
            acct,
            "conversation",
            "conversation.disappearing.set",
            json!({ "conversation_id": context, "ttl_secs": body["seconds"].as_u64().unwrap_or(0) }),
        )),
        methods::DEVICE_REVOKE => Plan::Call {
            request: BusRequest::new(
                acct,
                "device",
                "device.revoke",
                json!({ "device_id": text(body, "device_id") }),
            ),
            refresh: vec![Fetch::DeviceList],
        },
        methods::SEND_MESSAGE => send_message(acct, body, known),
        methods::GROUP_KICK
        | methods::GROUP_BAN
        | methods::GROUP_REMOVE_MEMBER
        | methods::GROUP_SET_ROLE => governance(acct, method, body, known),
        methods::CONTACT_ADD
        | methods::CONTACT_ACCEPT
        | methods::CONTACT_REJECT
        | methods::CONTACT_BLOCK
        | methods::CONTACT_UNBLOCK
        | methods::CONTACT_REMOVE => contact::contact_call(acct, method, body, known),
        _ => refuse_invalid("no daemon counterpart"),
    }
}

fn send_message(acct: Option<&str>, body: &Value, known: &impl Known) -> Plan {
    let context = ContextId::new(text(body, "context_id"));
    match known.context_kind(&context) {
        Some(ContextKind::Group) => {}
        Some(kind) => {
            return Plan::Refuse {
                code: FailureCode::UnsupportedByBackend,
                message: capabilities::reason_for(&methods::in_context(
                    methods::SEND_MESSAGE,
                    kind,
                ))
                .unwrap_or(capabilities::reason::DIRECT_ROUTE)
                .to_owned(),
            };
        }
        None => return refuse_invalid("unknown conversation"),
    }
    let Some(sender) = known.self_device() else {
        return refuse_invalid("this device is not known yet");
    };
    let plaintext = ramflux_protocol::encode_base64url(text(body, "text").as_bytes());
    let message_id = format!(
        "desk-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    );
    Plan::Call {
        request: BusRequest::new(
            acct,
            "group",
            "group.send",
            json!({
                "group_id": context,
                "conversation_id": context,
                "message_id": message_id,
                "sender_id": sender,
                "encrypted_body_base64": "",
                "plaintext_body_base64": plaintext,
            }),
        ),
        refresh: vec![Fetch::Messages(context)],
    }
}

fn governance(acct: Option<&str>, method: &str, body: &Value, known: &impl Known) -> Plan {
    let Some(actor) = known.self_device() else {
        return refuse_invalid("this device is not known yet");
    };
    let group = text(body, "context_id");
    let member = text(body, "participant_id");
    let (daemon, extra) = match method {
        methods::GROUP_KICK => ("group.member.kick", None),
        methods::GROUP_BAN => ("group.member.ban", None),
        methods::GROUP_REMOVE_MEMBER => ("group.member.remove", None),
        _ => {
            let role = serde_json::from_value::<Role>(body["role"].clone()).ok();
            let role = match role {
                Some(Role::Owner) => "owner",
                Some(Role::Admin) => "admin",
                Some(Role::Member) => "member",
                Some(Role::Bot) => "bot",
                None => return refuse_invalid("unknown role"),
            };
            ("group.role.set", Some(role))
        }
    };
    let mut request = json!({ "group_id": group, "actor_id": actor, "member_id": member });
    if let (Some(role), Value::Object(map)) = (extra, &mut request) {
        map.insert("role".to_owned(), json!(role));
    }
    Plan::Call {
        request: BusRequest::new(acct, "group", daemon, request),
        refresh: vec![Fetch::GroupList],
    }
}
