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
//! - [`contact`]: contact effects and the links they name, and verification.
//! - [`contact_card`]: adding a contact from a card, exporting this account's.
//! - [`conversation`]: starting a direct conversation (its first message).
//! - [`message`]: direct and group sends.
//! - [`media`]: attachments: send, download, save, retry and cancel.
//! - [`group`]: membership, roles and invitations.
//! - [`surface`]: surface actions and update requests.
//! - [`card_session`]: the card runtime's session calls (`card.session.*`).
//! - [`group_create`]: creating and renaming a group.
//! - [`task`]: starting, accepting, declining, cancelling and resolving
//!   conversation tasks.
//! - [`backup`]: full backup and restore jobs.
//! - [`agents`]: named agents: creation, persona, access, jobs and approvals,
//!   and the PRX runtime on this computer (in process, no bus call).
//! - [`call`]: placing, answering, declining and hanging up voice calls.

pub mod agents;
pub mod backup;
pub mod call;
pub mod card_session;
pub mod contact;
pub mod contact_card;
pub mod conversation;
pub mod fetch;
pub mod group;
pub mod group_create;
pub mod media;
pub mod message;
pub mod surface;
pub mod task;
pub mod username;

use ramflux_desktop_core::{
    AccountId, AgentRuntimeAction, ContextId, ContextKind, Effect, FailureCode, MomentId, ObjectId,
    PinSource, SecretInput, methods,
};
use serde_json::{Value, json};

use crate::bus::{BusRequest, idempotency};
use crate::capabilities;
use crate::directory::Directory;
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
    /// What the client directories listed (routes, devices, commitments).
    fn directory(&self) -> Option<&Directory> {
        None
    }
    /// Where a reported surface lives, by its desktop id (`surface_ref`).
    fn surface(&self, surface_ref: &str) -> Option<&crate::surface::SurfaceTarget> {
        let _ = surface_ref;
        None
    }
    /// The name this account keeps for a conversation (a rename on its way
    /// first), which group calls carry.
    fn kept_title(&self, context: &ContextId) -> Option<&str> {
        self.directory().and_then(|d| d.title_of(context))
    }
    /// Where a reported task lives, by its desktop id.
    fn task(&self, task_id: &str) -> Option<&task::TaskTarget> {
        let _ = task_id;
        None
    }
    /// What a reply, edit or delete of a listed message names.
    fn message_target(&self, moment: &MomentId) -> Option<&message::MessageTarget> {
        let _ = moment;
        None
    }
    /// What is known about an attachment.
    fn media(&self, object: &ObjectId) -> Option<&media::MediaRef> {
        let _ = object;
        None
    }
    /// The private copy of a downloaded (or sent) attachment.
    fn media_file(&self, object: &ObjectId) -> Option<&std::path::Path> {
        let _ = object;
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
    /// Requests in order (one per member device); the first failure stops
    /// the rest and is reported with how many were applied.
    Calls {
        /// The requests.
        requests: Vec<BusRequest>,
        /// Reads after the last one, or after a failure.
        refresh: Vec<Fetch>,
    },
    /// An attachment job, run step by step (see [`media`]).
    Media(media::Job),
    /// `contact.verify`: first `lookup` (`contact.safety_number`), then the
    /// verification of the compared snapshot it answers (`expected`).
    Verify {
        /// The safety number read.
        lookup: BusRequest,
        /// Reads after the verification.
        refresh: Vec<Fetch>,
    },
    /// A desktop-local preference; the translator echoes it.
    Local,
    /// A username claim or change, run as a job (see [`username`]).
    Username(Box<username::ClaimJob>),
    /// Write the redacted diagnostics bundle to `path`.
    Diagnostics {
        /// Destination chosen by the user.
        path: String,
    },
    /// Run a PRX runtime action on this computer, in process. The provider
    /// key is handed only to PRX's key callback.
    PrxHost {
        /// The action.
        action: AgentRuntimeAction,
        /// The provider (install only).
        provider: String,
        /// The model (install only).
        model: String,
        /// The provider key (install only).
        key: Option<SecretInput>,
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
    if let Effect::AgentRuntime { action, provider, model, key, nonce } = effect {
        return Planned {
            method: agents::RUNTIME_METHOD.to_owned(),
            body: json!({
                "action": agents::runtime_action_name(action),
                "provider": provider,
                "nonce": nonce,
            }),
            plan: Plan::PrxHost { action, provider, model, key },
        };
    }
    let (method, body, secrets) = match effect {
        Effect::Call { method, body } => (method, body, Vec::new()),
        Effect::SecretCall { method, body, secrets } => (method, body, secrets),
        Effect::RequireConfirmation { .. } | Effect::AgentRuntime { .. } => {
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
    let key = |request: BusRequest| -> Result<BusRequest, String> {
        if idempotency::is_mutation(&request.method) {
            Ok(request.with_operation_id(idempotency::fresh_key()?))
        } else {
            Ok(request)
        }
    };
    match plan {
        Plan::Call { request, refresh } => match key(request) {
            Ok(request) => Plan::Call { request, refresh },
            Err(message) => refuse_invalid(&message),
        },
        Plan::Calls { requests, refresh } => {
            match requests.into_iter().map(key).collect::<Result<Vec<_>, _>>() {
                Ok(requests) => Plan::Calls { requests, refresh },
                Err(message) => refuse_invalid(&message),
            }
        }
        other => other,
    }
}

/// Refuses with a reason key (translated by the UI).
pub(crate) fn refuse(reason: &str) -> Plan {
    Plan::Refuse { code: FailureCode::UnsupportedByBackend, message: reason.to_owned() }
}

pub(crate) fn refuse_invalid(message: &str) -> Plan {
    Plan::Refuse { code: FailureCode::Daemon("invalid".to_owned()), message: message.to_owned() }
}

pub(crate) fn text<'a>(body: &'a Value, field: &str) -> &'a str {
    body.get(field).and_then(Value::as_str).unwrap_or_default()
}

pub(crate) fn call(request: BusRequest) -> Plan {
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
            // Always sent with its passphrase: a passphrase-less unlock would
            // let the daemon prompt on this connection's attended provider.
            // The app lock's state is read back with the unlock.
            Some(secret) => Plan::Call {
                request: BusRequest::new(
                    Some(text(body, "account_id")),
                    "account",
                    "account.unlock",
                    json!({}),
                )
                .with_secret("passphrase", secret),
                refresh: vec![Fetch::AppLock(AccountId::new(text(body, "account_id")))],
            },
            None => refuse_invalid("missing passphrase"),
        },
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
        methods::APP_LOCK_CONFIGURE => call(BusRequest::new(
            Some(text(body, "account_id")),
            "account",
            "account.app_lock.configure",
            json!({ "enabled": body["enabled"], "timeout_ms": body["timeout_ms"] }),
        )),
        methods::APP_LOCK_LIFECYCLE => call(BusRequest::new(
            Some(text(body, "account_id")),
            "account",
            "account.app_lock.lifecycle",
            json!({ "state": body["state"] }),
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
        methods::BACKUP_CREATE
        | methods::RESTORE_VERIFY
        | methods::RESTORE_INSTALL
        | methods::BACKUP_CANCEL => backup::backup_effect(acct, method, body, secrets),
        _ if agents::is_agent_method(method) => agents::agent_effect(method, body, account),
        methods::DIAGNOSTICS_EXPORT => Plan::Diagnostics { path: text(body, "path").to_owned() },
        methods::CONTEXT_PIN | methods::CONTEXT_UNPIN => Plan::Local,
        _ if acct.is_none() => refuse_invalid("this action needs an unlocked account"),
        methods::USERNAME_CLAIM => username::claim(acct, body, std::mem::take(&mut secrets)),
        method if card_session::is_card_method(method) => {
            match card_session::request(acct, method, body, std::mem::take(&mut secrets)) {
                Some(request) => Plan::Call { request, refresh: Vec::new() },
                None => refuse_invalid("the card call does not follow the contract"),
            }
        }
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
        methods::SEND_MESSAGE => message::send(acct, body, known),
        methods::MESSAGE_REPLY | methods::MESSAGE_EDIT | methods::MESSAGE_DELETE => {
            message::mutate(acct, method, body, known)
        }
        methods::GROUP_LEAVE => group::leave(acct, body, known),
        methods::GROUP_INVITATION_ACCEPT | methods::GROUP_INVITATION_DECLINE => {
            group::invitation(acct, method, body)
        }
        methods::USERNAME_CONTACT_ADD => contact::username_add(acct, body),
        methods::USERNAME_RESOLVE => username::resolve(acct, body),
        methods::CONVERSATION_START => conversation::start(acct, body, known),
        methods::ATTACH_OBJECT => media::send(body, known),
        methods::OBJECT_PULL
        | methods::OBJECT_SAVE
        | methods::OBJECT_RETRY
        | methods::OBJECT_CANCEL => media::object_call(method, body, known),
        methods::GROUP_KICK
        | methods::GROUP_REMOVE_MEMBER
        | methods::GROUP_SET_ROLE
        | methods::GROUP_ADD_MEMBER => group::governance(acct, method, body, known),
        methods::SAFETY_VERIFY => contact::verify(acct, body, known),
        methods::SURFACE_REQUEST_RESYNC => surface::request_resync(acct, body, known),
        methods::CONTACT_CARD_IMPORT => contact_card::import(acct, body),
        methods::CONTACT_CARD_EXPORT => contact_card::export(acct),
        methods::GROUP_CREATE => group_create::create(acct, body, known),
        methods::CALL_INVITE
        | methods::CALL_ANSWER
        | methods::CALL_REJECT
        | methods::CALL_HANGUP => call::call_effect(acct, method, body),
        methods::CONTEXT_RENAME => group_create::rename(acct, body, known),
        methods::TASK_START | methods::TASK_CANCEL | methods::TASK_RESOLVE => {
            task::task_call(acct, method, body, known)
        }
        methods::CONTACT_REQUEST_ACCEPT
        | methods::CONTACT_REQUEST_DECLINE
        | methods::CONTACT_REQUEST_BLOCK
        | methods::CONTACT_REQUESTS_POLICY => contact::request_call(acct, method, body),
        methods::CONTACT_ADD
        | methods::CONTACT_ACCEPT
        | methods::CONTACT_REJECT
        | methods::CONTACT_BLOCK
        | methods::CONTACT_UNBLOCK
        | methods::CONTACT_REMOVE => contact::contact_call(acct, method, body, known),
        _ => refuse_invalid("no daemon counterpart"),
    }
}
