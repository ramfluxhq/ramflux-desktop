// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Group governance: membership, roles and invitations.
//!
//! A desktop participant is a principal; a group member is one of its
//! devices. The group is the one the conversation names in
//! `conversation.directory` (never the conversation id itself).
//!
//! - Remove, kick, set role: one request per member device of the
//!   participant in the group (`group.list` member routes), acting as this
//!   device.
//! - Add (`group.member.add`): one request per verified device of the
//!   participant (`identity.directory`: signing key and delivery target),
//!   with the participant's commitment, so the member can read the group.
//!   A participant without a verified device is refused with that reason.
//!
//! Several requests run in order and stop at the first failure.

use ramflux_desktop_core::{ContextId, Role, methods};
use ramflux_sdk::{
    LocalBusGroupMemberAddRequest, LocalBusGroupMemberKickRequest,
    LocalBusGroupMemberRemoveRequest, LocalBusGroupRoleSetRequest,
};
use serde::Serialize;
use serde_json::Value;

use super::fetch::{Fetch, Paged};
use super::{Known, Plan, refuse, refuse_invalid, text};
use crate::bus::BusRequest;
use crate::directory::Directory;
use crate::translate::reason::key;

fn role_name(body: &Value) -> Option<&'static str> {
    match serde_json::from_value::<Role>(body["role"].clone()).ok()? {
        Role::Owner => Some("owner"),
        Role::Admin => Some("admin"),
        Role::Member => Some("member"),
        Role::Bot => Some("bot"),
    }
}

/// Plans a governance effect.
pub(super) fn governance(
    acct: Option<&str>,
    method: &str,
    body: &Value,
    known: &impl Known,
) -> Plan {
    let context = ContextId::new(text(body, "context_id"));
    let participant = text(body, "participant_id");
    let Some(directory) = known.directory() else { return refuse(key::NO_GROUP_LINK) };
    let Some(group) = directory.group_of(&context).map(str::to_owned) else {
        return refuse(key::NO_GROUP_LINK);
    };
    let Some(actor) = known.self_device().map(str::to_owned) else { return refuse(key::NO_SELF) };
    let role = role_name(body);
    let requests = match method {
        methods::GROUP_ADD_MEMBER => {
            let Some(role) = role else { return refuse_invalid("unknown role") };
            invitations(directory, &group, participant, role)
        }
        _ => members(directory, &group, &actor, participant, method, role),
    };
    let requests = match requests {
        Ok(bodies) if bodies.is_empty() => return refuse(key::NOT_A_MEMBER_DEVICE),
        Ok(bodies) => bodies,
        Err(why) => return why.plan(),
    };
    let daemon = match method {
        methods::GROUP_ADD_MEMBER => "group.member.add",
        methods::GROUP_KICK => "group.member.kick",
        methods::GROUP_REMOVE_MEMBER => "group.member.remove",
        _ => "group.role.set",
    };
    let mut requests: Vec<BusRequest> =
        requests.into_iter().map(|body| BusRequest::new(acct, "group", daemon, body)).collect();
    let refresh = vec![Fetch::first(Paged::Groups), Fetch::first(Paged::Conversations)];
    if requests.len() == 1 {
        Plan::Call { request: requests.remove(0), refresh }
    } else {
        Plan::Calls { requests, refresh }
    }
}

/// Why requests could not be built.
pub(super) enum Why {
    /// A reason key.
    Reason(&'static str),
    /// A malformed request.
    Invalid(String),
}

impl Why {
    pub(super) fn plan(self) -> Plan {
        match self {
            Self::Reason(reason) => refuse(reason),
            Self::Invalid(message) => refuse_invalid(&message),
        }
    }
}

fn body(value: impl Serialize) -> Result<Value, Why> {
    serde_json::to_value(value).map_err(|error| Why::Invalid(error.to_string()))
}

/// Remove, kick or set the role of each member device.
fn members(
    directory: &Directory,
    group: &str,
    actor: &str,
    participant: &str,
    method: &str,
    role: Option<&str>,
) -> Result<Vec<Value>, Why> {
    let devices = directory.member_devices(group, participant);
    devices
        .into_iter()
        .map(|member| {
            let (group_id, actor_id) = (group.to_owned(), actor.to_owned());
            match method {
                methods::GROUP_KICK => body(LocalBusGroupMemberKickRequest {
                    group_id,
                    actor_id,
                    member_id: member,
                    reason: None,
                }),
                methods::GROUP_REMOVE_MEMBER => {
                    body(LocalBusGroupMemberRemoveRequest { group_id, actor_id, member_id: member })
                }
                _ => match role {
                    Some(role) => body(LocalBusGroupRoleSetRequest {
                        group_id,
                        actor_id,
                        member_id: member,
                        role: role.to_owned(),
                    }),
                    None => Err(Why::Invalid("unknown role".to_owned())),
                },
            }
        })
        .collect()
}

/// Add each verified device of the participant.
pub(super) fn invitations(
    directory: &Directory,
    group: &str,
    participant: &str,
    role: &str,
) -> Result<Vec<Value>, Why> {
    let Some(identity) = directory.identities.get(participant) else {
        return Err(Why::Reason(key::NO_MEMBER_ROUTE));
    };
    let devices: Vec<_> = identity.devices.iter().filter(|d| d.routable()).collect();
    if devices.is_empty() || identity.commitment.is_none() {
        return Err(Why::Reason(key::NO_MEMBER_ROUTE));
    }
    devices
        .into_iter()
        .map(|device| {
            let key = device.signing_key.clone().unwrap_or_default();
            let target = device.target.clone().unwrap_or_default();
            body(LocalBusGroupMemberAddRequest {
                group_id: group.to_owned(),
                member_id: device.device_id.clone(),
                role: role.to_owned(),
                member_signing_public_key: Some(key),
                member_principal_commitment: identity.commitment.clone(),
                target_delivery_id: Some(target),
                federation: None,
            })
        })
        .collect()
}
