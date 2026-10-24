// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Policy for creating and renaming a group.
//!
//! Only the attended human creates a group. Its conversation id is fresh
//! (never a known context), its name is not blank, and every first member
//! is an accepted contact that a conversation could be started with now
//! (not this account, no unresolved key change, reachable): the daemon adds
//! a member by their verified devices, so anyone else would be refused or
//! could not read the group. Members are added once each, in the order
//! given. A group conversation can be renamed; a direct one is named after
//! the contact.

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{Decision, IntentError, Policy, check_start_conversation};
use crate::ids::{ContextId, ParticipantId};
use crate::intent::{IntentKind, methods};
use crate::model::ContextKind;
use crate::store::Store;

/// Longest group name, in characters.
pub const MAX_GROUP_TITLE_LEN: usize = 100;

/// Most members added when a group is created (more can be added later).
pub const MAX_GROUP_MEMBERS: usize = 64;

/// Why a new group's name or member list cannot be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GroupProblem {
    /// The name is blank.
    EmptyTitle,
    /// The name is longer than [`MAX_GROUP_TITLE_LEN`] characters.
    TitleTooLong,
    /// The name contains control characters.
    TitleControl,
    /// No member was chosen.
    NoMembers,
    /// More than [`MAX_GROUP_MEMBERS`] members were chosen.
    TooManyMembers,
}

/// Checks a group name (trimmed); returns it trimmed.
///
/// # Errors
///
/// [`IntentError::InvalidGroup`].
pub fn check_group_title(title: &str) -> Result<&str, IntentError> {
    let title = title.trim();
    if title.is_empty() {
        Err(IntentError::InvalidGroup(GroupProblem::EmptyTitle))
    } else if title.chars().count() > MAX_GROUP_TITLE_LEN {
        Err(IntentError::InvalidGroup(GroupProblem::TitleTooLong))
    } else if title.chars().any(char::is_control) {
        Err(IntentError::InvalidGroup(GroupProblem::TitleControl))
    } else {
        Ok(title)
    }
}

/// Checks a new group's name and first members before it is created; the
/// new-group form runs the same check to explain a refusal in place.
/// Returns the trimmed name and the members, each once.
///
/// # Errors
///
/// [`IntentError::InvalidGroup`], or the first member that cannot be added
/// with the reason [`check_start_conversation`] gives.
pub fn check_create_group<'a>(
    store: &Store,
    title: &'a str,
    members: &[ParticipantId],
) -> Result<(&'a str, Vec<ParticipantId>), IntentError> {
    let title = check_group_title(title)?;
    let mut unique: Vec<ParticipantId> = Vec::with_capacity(members.len());
    for member in members {
        if !unique.contains(member) {
            unique.push(member.clone());
        }
    }
    if unique.is_empty() {
        return Err(IntentError::InvalidGroup(GroupProblem::NoMembers));
    }
    if unique.len() > MAX_GROUP_MEMBERS {
        return Err(IntentError::InvalidGroup(GroupProblem::TooManyMembers));
    }
    for member in &unique {
        check_start_conversation(store, member)?;
    }
    Ok((title, unique))
}

fn fresh(store: &Store, context_id: &ContextId) -> Result<(), IntentError> {
    if context_id.as_str().trim().is_empty() {
        return Err(IntentError::Empty("conversation id"));
    }
    if store.context(context_id).is_some() {
        return Err(IntentError::ContextExists(context_id.clone()));
    }
    Ok(())
}

/// Decides group-creation kinds; `None` for other kinds.
pub(super) fn decide(p: &Policy<'_>, kind: &IntentKind) -> Option<Result<Decision, IntentError>> {
    Some(match kind {
        IntentKind::CreateGroup { context_id, title, members } => {
            p.human_only("create a group").and_then(|()| {
                fresh(p.store, context_id)?;
                let (title, members) = check_create_group(p.store, title, members)?;
                Ok(Decision::Call(
                    methods::GROUP_CREATE,
                    json!({ "context_id": context_id, "title": title, "members": members }),
                ))
            })
        }
        IntentKind::RenameContext { context_id, title } => {
            p.context(context_id).and_then(|context| {
                if context.kind != ContextKind::Group {
                    return Err(IntentError::NotAGroup(context_id.clone()));
                }
                let title = check_group_title(title)?;
                Ok(Decision::Call(
                    methods::CONTEXT_RENAME,
                    json!({ "context_id": context_id, "title": title }),
                ))
            })
        }
        _ => return None,
    })
}
