// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Conversations and groups as contexts, their pins and settings.
//!
//! A context is a conversation of `conversation.directory`. Its kind,
//! title, participants (principals), unread count, mute and disappearing
//! timer are the row's; a null title shows the conversation id. A group
//! conversation names its group (`group_id`); that group's row in
//! `group.list` gives the epoch, the roles (by member device, shown on the
//! member's principal when all its devices agree) and the member routes.
//! A member whose principal the daemon does not know is shown by its device
//! id. A group with no linked conversation has no context: nothing is
//! derived from its id. A row without a classification and without a group
//! is a direct conversation (the daemon classifies every group
//! conversation it links). Archived and hidden conversations are not shown.

use std::collections::BTreeMap;

use ramflux_desktop_core::{
    Context, ContextId, ContextKind, DomainEvent, MomentId, ParticipantId, Role,
};
use serde_json::Value;

use super::{Translated, Translator};
use crate::directory::{Conversation, Group, field};
use crate::plan::fetch::Fetch;

/// Moment id of a stored message.
pub(crate) fn message_moment_id(context: &ContextId, message: &str) -> MomentId {
    MomentId::new(format!("msg:{context}:{message}"))
}

fn role(name: &str) -> Option<Role> {
    match name {
        "owner" => Some(Role::Owner),
        "admin" => Some(Role::Admin),
        "member" => Some(Role::Member),
        "bot" => Some(Role::Bot),
        _ => None,
    }
}

impl Translator {
    /// Emits the context when it differs from what was reported.
    pub(super) fn put_context(&mut self, context: Context) -> Vec<DomainEvent> {
        if self.contexts.get(&context.id) == Some(&context) {
            return Vec::new();
        }
        self.contexts.insert(context.id.clone(), context.clone());
        vec![DomainEvent::ContextUpserted(context)]
    }

    fn remove_context(&mut self, id: &ContextId) -> DomainEvent {
        // A removed context keeps its pin: should it come back, it is
        // restored pinned.
        self.contexts.remove(id);
        let prefix = message_moment_id(id, "");
        self.moments.retain(|moment, _| !moment.as_str().starts_with(prefix.as_str()));
        DomainEvent::ContextRemoved { context_id: id.clone() }
    }

    /// A complete `group.list`.
    pub(super) fn groups(&mut self, rows: &[Value]) -> Vec<DomainEvent> {
        self.directory.groups = rows
            .iter()
            .filter_map(|row| Some((field(row, "group_id").or_else(|| field(row, "id"))?, row)))
            .map(|(id, row)| (id, Group::from_row(row)))
            .collect();
        self.rebuild_contexts().events
    }

    /// A complete `conversation.directory`.
    pub(super) fn conversations(&mut self, rows: &[Value]) -> Translated {
        self.directory.conversations = rows
            .iter()
            .filter_map(|row| {
                Some((ContextId::new(field(row, "id")?), Conversation::from_row(row)))
            })
            .collect();
        self.conversation_rows = Some(rows.to_vec());
        self.rebuild_contexts()
    }

    /// Contexts from the latest conversation rows and groups; removes the
    /// ones no longer listed.
    fn rebuild_contexts(&mut self) -> Translated {
        let mut out = Translated::default();
        let Some(rows) = self.conversation_rows.clone() else { return out };
        let mut listed = Vec::new();
        for row in &rows {
            let Some(id) = field(row, "id").map(ContextId::new) else { continue };
            if row["archived"].as_bool().unwrap_or(false)
                || row["hidden"].as_bool().unwrap_or(false)
            {
                continue;
            }
            let (context, named) = self.context_from_row(&id, row);
            for name in named {
                out.events.extend(self.named(&name));
            }
            let kind = context.kind;
            out.events.extend(self.put_context(context));
            if kind != ContextKind::Group
                && (self.pins.contains(&id) || self.focus.as_ref() == Some(&id))
            {
                out.follow.push(Fetch::Messages(id.clone()));
            }
            listed.push(id);
        }
        // A conversation this device started stays until the directory has
        // listed it once: a listing that was read before the start (and
        // answered after it) does not know it yet.
        self.started.retain(|id| !listed.contains(id));
        let gone: Vec<ContextId> = self
            .contexts
            .keys()
            .filter(|id| !listed.contains(id) && !self.started.contains(*id))
            .cloned()
            .collect();
        for id in gone {
            out.events.push(self.remove_context(&id));
        }
        out.events.extend(self.place_interventions());
        out
    }

    /// One context, and the participant ids it names.
    fn context_from_row(&self, id: &ContextId, row: &Value) -> (Context, Vec<String>) {
        let conversation = Conversation::from_row(row);
        let kind = match (conversation.kind, &conversation.group_id) {
            (Some(kind), _) => kind,
            (None, Some(_)) => ContextKind::Group,
            (None, None) => ContextKind::Direct,
        };
        let mut participants: Vec<String> = row["participants"]
            .as_array()
            .map(|ids| ids.iter().filter_map(Value::as_str).map(str::to_owned).collect())
            .unwrap_or_default();
        let group = conversation.group_id.as_ref().and_then(|g| self.directory.groups.get(g));
        let mut roles = BTreeMap::new();
        if let Some(group) = group {
            let mut by_member: BTreeMap<String, Vec<Option<Role>>> = BTreeMap::new();
            for member in &group.members {
                let name = member.principal_id.clone().unwrap_or_else(|| member.member_id.clone());
                if !participants.contains(&name) {
                    participants.push(name.clone());
                }
                let role = group.roles.get(&member.member_id).and_then(|r| role(r));
                by_member.entry(name).or_default().push(role);
            }
            for (name, found) in by_member {
                if let Some(Some(first)) = found.first()
                    && found.iter().all(|r| r.as_ref() == Some(first))
                {
                    roles.insert(ParticipantId::new(name.as_str()), *first);
                }
            }
        }
        let known = self.contexts.get(id);
        let context = Context {
            id: id.clone(),
            kind,
            title: field(row, "title").unwrap_or_else(|| id.to_string()),
            participants: participants.iter().map(|p| ParticipantId::new(p.as_str())).collect(),
            // Pins are restored in pin order once the lists are known.
            pinned: known.is_some_and(|c| c.pinned),
            unread: row["unread_count"]
                .as_u64()
                .map_or(0, |n| u32::try_from(n).unwrap_or(u32::MAX)),
            muted: row["muted"].as_bool().unwrap_or(false),
            disappearing_secs: row["disappearing"]["timer_seconds"]
                .as_u64()
                .and_then(|s| u32::try_from(s).ok())
                .filter(|s| *s > 0),
            roles,
            epoch: group.and_then(|g| g.epoch),
        };
        (context, participants)
    }

    fn add_pin(&mut self, context: &ContextId) {
        if !self.pins.contains(context) {
            self.pins.push(context.clone());
            self.pins_changed = true;
        }
    }

    /// Applies a desktop-local pin (the daemon has no pin method) and
    /// returns the settings event.
    pub(super) fn local_pin(&mut self, context: &ContextId, pinned: bool) -> Vec<DomainEvent> {
        if !self.contexts.contains_key(context) {
            return Vec::new();
        }
        if pinned {
            self.add_pin(context);
        } else if self.pins.contains(context) {
            self.pins.retain(|id| id != context);
            self.pins_changed = true;
        }
        self.show_pin(context, pinned)
    }

    /// Marks known, not yet pinned contexts of the persisted pins as pinned,
    /// in pin order. Called after the snapshot and when the lists change.
    pub fn restore_pins(&mut self) -> Vec<DomainEvent> {
        let due: Vec<ContextId> = self
            .pins
            .iter()
            .filter(|id| self.contexts.get(*id).is_some_and(|c| !c.pinned))
            .cloned()
            .collect();
        due.iter().flat_map(|id| self.show_pin(id, true)).collect()
    }

    fn show_pin(&mut self, context: &ContextId, pinned: bool) -> Vec<DomainEvent> {
        let Some(mut ctx) = self.contexts.get(context).cloned() else { return Vec::new() };
        ctx.pinned = pinned;
        self.contexts.insert(context.clone(), ctx);
        vec![DomainEvent::ContextSettingsChanged {
            context_id: context.clone(),
            pinned: Some(pinned),
            muted: None,
            disappearing: None,
        }]
    }

    /// Applies a confirmed context setting and returns its event.
    pub(super) fn context_setting(
        &mut self,
        context: &ContextId,
        muted: Option<bool>,
        disappearing: Option<ramflux_desktop_core::Disappearing>,
    ) -> Vec<DomainEvent> {
        let Some(mut ctx) = self.contexts.get(context).cloned() else { return Vec::new() };
        if let Some(muted) = muted {
            ctx.muted = muted;
        }
        if let Some(timer) = disappearing {
            ctx.disappearing_secs = match timer {
                ramflux_desktop_core::Disappearing::Off => None,
                ramflux_desktop_core::Disappearing::After(secs) => Some(secs),
            };
        }
        self.contexts.insert(context.clone(), ctx);
        vec![DomainEvent::ContextSettingsChanged {
            context_id: context.clone(),
            pinned: None,
            muted,
            disappearing,
        }]
    }
}
