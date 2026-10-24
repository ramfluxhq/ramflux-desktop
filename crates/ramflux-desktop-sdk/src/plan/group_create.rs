// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Creating a group and renaming a group conversation.
//!
//! The daemon has no "create a group conversation" request of its own: it
//! links a conversation to a group, with the name this account keeps for
//! it, from the group calls that name the conversation. So creating a group
//! runs, in order and stopping at the first failure:
//!
//! 1. `group.create` with a fresh group id, this device as the creator
//!    (owner) with its verified signing key and delivery target from this
//!    account's identity, and the new conversation's id and name;
//! 2. `group.member.add` for each verified device of each first member,
//!    with the member's commitment, so every member can read the group
//!    (the same requests as adding a member later).
//!
//! A member without a verified device refuses the whole creation before
//! anything is sent, naming the member. The group list and the
//! conversation directory are read afterwards, so the group shows as a
//! conversation.
//!
//! Renaming a group conversation is the group read (`group.receive`) of
//! that conversation carrying the new name: the daemon records the name
//! from it. The name is this account's; other members keep their own.

use ramflux_desktop_core::{ContextId, methods};
use ramflux_sdk::LocalBusGroupCreateRequest;
use serde_json::{Value, json};

use super::fetch::{Fetch, Paged};
use super::group::invitations;
use super::message::fresh_id;
use super::{Known, Plan, refuse, refuse_invalid, text};
use crate::bus::BusRequest;
use crate::translate::reason::{key, keyed};

/// Plans `group.create`: the group, its conversation, then its members.
pub(super) fn create(acct: Option<&str>, body: &Value, known: &impl Known) -> Plan {
    let context = ContextId::new(text(body, "context_id"));
    let title = text(body, "title").trim();
    if context.as_str().is_empty() || title.is_empty() {
        return refuse_invalid("no conversation or name given");
    }
    if known.context_kind(&context).is_some() {
        return refuse_invalid("the conversation already exists");
    }
    let (Some(directory), Some(device)) = (known.directory(), known.self_device()) else {
        return refuse(key::NO_SELF);
    };
    let group = match fresh_id("group-") {
        Ok(id) => id,
        Err(error) => return refuse_invalid(&error),
    };
    let own = directory.own_device(device);
    let create = LocalBusGroupCreateRequest {
        group_id: group.clone(),
        creator_id: device.to_owned(),
        creator_signing_public_key: own.and_then(|d| d.signing_key.clone()),
        creator_target_delivery_id: own.and_then(|d| d.target.clone()),
    };
    let mut create = match serde_json::to_value(create) {
        Ok(value) => value,
        Err(error) => return refuse_invalid(&error.to_string()),
    };
    create["conversation_id"] = json!(context);
    create["title"] = json!(title);
    let mut requests = vec![BusRequest::new(acct, "group", "group.create", create)];
    let members = body["members"].as_array().map(Vec::as_slice).unwrap_or_default();
    for member in members.iter().filter_map(Value::as_str) {
        match invitations(directory, &group, device, member, methods::GROUP_ADD_MEMBER, "member") {
            Ok(adds) if !adds.is_empty() => requests.extend(
                adds.into_iter().map(|add| BusRequest::new(acct, "group", "group.member.add", add)),
            ),
            Ok(_) | Err(super::group::Why::Reason(_)) => {
                return refuse(&keyed(key::GROUP_MEMBER_ROUTE, &[("name", member)]));
            }
            Err(why) => return why.plan(),
        }
    }
    Plan::Calls {
        requests,
        refresh: vec![Fetch::first(Paged::Groups), Fetch::first(Paged::Conversations)],
    }
}

/// Plans `conversation.rename` of a group conversation.
pub(super) fn rename(acct: Option<&str>, body: &Value, known: &impl Known) -> Plan {
    let context = ContextId::new(text(body, "context_id"));
    let title = text(body, "title").trim();
    if title.is_empty() {
        return refuse_invalid("no name given");
    }
    let Some(group) = known.directory().and_then(|d| d.group_of(&context)) else {
        return refuse(key::NO_GROUP_LINK);
    };
    let read = Fetch::GroupReceive {
        context: context.clone(),
        group: group.to_owned(),
        title: Some(title.to_owned()),
    };
    Plan::Call { request: read.request(acct), refresh: vec![Fetch::first(Paged::Conversations)] }
}
