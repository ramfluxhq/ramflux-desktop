// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Call controls of message schema v1, built only with the generated types.
//! Every action carries recognisable key material so tests can prove it never
//! reaches a model.

use ramflux_protocol::message::{
    Call, CallAction, CallActionAck, CallActionAnswer, CallActionAnsweredElsewhere, CallActionBusy,
    CallActionFailed, CallActionHangup, CallActionInvite, CallActionReady, CallActionReject,
    CallActionRekey, CallActionRekeyAck, CallActionRinging, CallActionTimeout, CallControl,
    CallEndpoint, Content, EventIdentity, MessageUpdate, VoiceLeaseConsent, VoicePeerAllocation,
    call_action::Value as Action, content::Value, message_update::State,
};

/// The key material every fixture action carries.
pub const SECRET_KEY: &str = "SECRET-BASE-KEY-0xC0FFEE";

/// The thirteen actions, in the schema's order, with their wire names.
#[must_use]
pub fn all_actions() -> Vec<(&'static str, CallAction)> {
    let key = || SECRET_KEY.to_owned();
    [
        ("invite", Action::Invite(CallActionInvite { base_key: key() })),
        ("ringing", Action::Ringing(CallActionRinging {})),
        ("answer", Action::Answer(CallActionAnswer { base_key: key() })),
        ("ack", Action::Ack(CallActionAck { allocation: Some(VoicePeerAllocation::default()) })),
        ("ready", Action::Ready(CallActionReady { consent: Some(VoiceLeaseConsent::default()) })),
        ("reject", Action::Reject(CallActionReject {})),
        ("busy", Action::Busy(CallActionBusy {})),
        ("hangup", Action::Hangup(CallActionHangup {})),
        ("timeout", Action::Timeout(CallActionTimeout {})),
        ("failed", Action::Failed(CallActionFailed {})),
        ("answered_elsewhere", Action::AnsweredElsewhere(CallActionAnsweredElsewhere {})),
        ("rekey", Action::Rekey(CallActionRekey { generation: 1, base_key: key() })),
        ("rekey_ack", Action::RekeyAck(CallActionRekeyAck { generation: 1, base_key: key() })),
    ]
    .into_iter()
    .map(|(name, value)| (name, CallAction { value: Some(value) }))
    .collect()
}

/// An action by wire name.
///
/// # Panics
/// On a name that is not one of the thirteen.
#[must_use]
pub fn action(name: &str) -> CallAction {
    all_actions().into_iter().find(|(n, _)| *n == name).expect("a known call action").1
}

/// A rekey or ack of `generation`.
#[must_use]
pub fn rekey(generation: u64, ack: bool) -> CallAction {
    let value = if ack {
        Action::RekeyAck(CallActionRekeyAck { generation, base_key: SECRET_KEY.to_owned() })
    } else {
        Action::Rekey(CallActionRekey { generation, base_key: SECRET_KEY.to_owned() })
    };
    CallAction { value: Some(value) }
}

/// An endpoint of `principal`.
#[must_use]
pub fn endpoint(principal: &str) -> CallEndpoint {
    CallEndpoint {
        principal_id: principal.to_owned(),
        root_public_key: format!("root-{principal}"),
        device_id: format!("dev-{principal}"),
    }
}

/// One call between `caller` and `callee` in `conversation`.
#[derive(Debug, Clone)]
pub struct CallSpec {
    /// Conversation.
    pub conversation: String,
    /// Call id.
    pub call_id: String,
    /// Caller principal.
    pub caller: String,
    /// Callee principal.
    pub callee: String,
}

impl CallSpec {
    /// A call.
    #[must_use]
    pub fn new(conversation: &str, call_id: &str, caller: &str, callee: &str) -> Self {
        Self {
            conversation: conversation.to_owned(),
            call_id: call_id.to_owned(),
            caller: caller.to_owned(),
            callee: callee.to_owned(),
        }
    }

    /// A control from `sender` with sequence `seq` at `created_at` seconds.
    #[must_use]
    pub fn control(
        &self,
        identity: EventIdentity,
        sender: &str,
        seq: u64,
        created_at: i64,
        body: CallAction,
    ) -> MessageUpdate {
        let control = CallControl {
            schema: "ramflux.call.control.v1".to_owned(),
            call_id: self.call_id.clone(),
            conversation_id: self.conversation.clone(),
            caller: Some(endpoint(&self.caller)),
            callee: Some(endpoint(&self.callee)),
            sender: Some(endpoint(sender)),
            seq,
            created_at,
            body: Some(body),
            signature: "SECRET-SIGNATURE".to_owned(),
        };
        MessageUpdate {
            account_id: crate::v1_fixtures::ACCOUNT.to_owned(),
            journal_sequence: identity.device_sequence,
            identity: Some(identity),
            state: Some(State::Content(Content {
                value: Some(Value::Call(Call { control: Some(control) })),
            })),
        }
    }
}
