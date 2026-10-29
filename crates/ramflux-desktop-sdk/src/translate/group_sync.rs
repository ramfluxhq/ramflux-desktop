// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Group changes that arrive by sync, as timeline records (GRP-SYNC-01).
//!
//! A shared group has one signed state: its roster and its name. When the
//! creator's device (or an admin request the creator applied) changes it,
//! every member's daemon adopts the new state and announces `group.changed`;
//! `group.list` then lists the new roster (`member_routes`, each with the
//! member's `principal_id` when the daemon knows it) and the signed `name`
//! with its `name_epoch`. The desktop compares the listing with the one
//! before and says what changed in the group's conversation:
//!
//! | Change between two listings | Record |
//! |---|---|
//! | a principal now in the roster | [`GovernanceAction::Joined`] |
//! | a principal no longer in the roster | [`GovernanceAction::Removed`] |
//! | a different signed name (both known) | [`MomentKind::GroupRename`] |
//!
//! The first complete listing is the baseline: it reports nothing. A member
//! the daemon cannot name (no `principal_id`) is not reported, because the
//! record would name a device id nobody chose; the roster still changes.
//! Another device of a principal already in the group changes nothing. A
//! record is reported once: its id carries the group's epoch (or the name
//! epoch), so replays and repeated listings do not repeat it.

use std::collections::{BTreeMap, BTreeSet};

use ramflux_desktop_core::{
    ContextId, DomainEvent, GovernanceAction, GovernancePayload, GroupRenamePayload, Lifecycle,
    Moment, MomentId, MomentKind, ParticipantId, Timestamp,
};

use super::Translator;
use crate::directory::Group;

impl Group {
    /// The principals in the roster (devices of one principal count once).
    fn principals(&self) -> BTreeSet<&str> {
        self.members.iter().filter_map(|m| m.principal_id.as_deref()).collect()
    }
}

impl Translator {
    /// The records of what changed between `before` and the groups now
    /// listed. `None` for `before` is the baseline listing: nothing changed.
    pub(super) fn group_sync_moments(
        &mut self,
        before: Option<&BTreeMap<String, Group>>,
        now: Timestamp,
    ) -> Vec<DomainEvent> {
        let Some(before) = before else { return Vec::new() };
        let after = self.directory.groups.clone();
        let mut events = Vec::new();
        for (group_id, group) in &after {
            let Some(previous) = before.get(group_id) else { continue };
            let Some(context) = self.group_context(group_id) else { continue };
            let stamp = group.epoch.map_or_else(|| now.0.to_string(), |e| e.to_string());
            let was = previous.principals();
            let is = group.principals();
            for (action, who) in [
                (GovernanceAction::Joined, is.difference(&was)),
                (GovernanceAction::Removed, was.difference(&is)),
            ] {
                for principal in who {
                    let target = ParticipantId::new(*principal);
                    let id = format!("group-sync:{context}:{action:?}:{principal}:{stamp}");
                    events.extend(self.report_moment(Moment {
                        id: MomentId::new(id),
                        context_id: context.clone(),
                        origin: target.clone(),
                        at: now,
                        kind: MomentKind::Governance(GovernancePayload {
                            action,
                            target,
                            role: None,
                        }),
                        lifecycle: Lifecycle::Done,
                    }));
                }
            }
            if let (Some(old), Some(new)) = (&previous.name, &group.name)
                && old != new
            {
                let epoch = group.name_epoch.map_or_else(|| stamp.clone(), |e| e.to_string());
                let origin = self.directory.self_principal.clone().unwrap_or_default();
                events.extend(self.report_moment(Moment {
                    id: MomentId::new(format!("group-sync:{context}:name:{epoch}")),
                    context_id: context.clone(),
                    origin: ParticipantId::new(origin),
                    at: now,
                    kind: MomentKind::GroupRename(GroupRenamePayload {
                        old_name: old.clone(),
                        new_name: new.clone(),
                    }),
                    lifecycle: Lifecycle::Done,
                }));
            }
        }
        events
    }

    /// The context of the conversation linked to `group_id`, when shown.
    fn group_context(&self, group_id: &str) -> Option<ContextId> {
        self.directory
            .conversations
            .iter()
            .find(|(_, c)| c.group_id.as_deref() == Some(group_id))
            .map(|(id, _)| id.clone())
            .filter(|id| self.contexts.contains_key(id))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::translate::Scope;

    fn group(epoch: u64, name: &str, members: &[(&str, Option<&str>)]) -> serde_json::Value {
        let routes: Vec<_> = members
            .iter()
            .map(|(device, principal)| {
                json!({ "member_id": device, "principal_id": principal,
                        "delivery_target_known": true })
            })
            .collect();
        json!({ "group_id": "g-1", "group_epoch": epoch, "name": name,
                "name_epoch": epoch, "member_routes": routes })
    }

    fn translator() -> Translator {
        let mut t = Translator::new(Scope::Daemon, "rfd.sock");
        let rows = [json!({ "id": "thread", "kind": "group", "group_id": "g-1", "title": "T" })];
        t.groups(
            &[group(1, "Checklist", &[("d-me", Some("me")), ("d-ana", Some("ana"))])],
            Timestamp(0),
        );
        t.conversations(&rows);
        t
    }

    fn records(events: &[DomainEvent]) -> Vec<MomentKind> {
        events
            .iter()
            .filter_map(|e| match e {
                DomainEvent::MomentAdded(m) => Some(m.kind.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_first_listing_reports_nothing_then_roster_and_name_changes_are_reported_once() {
        let mut t = translator();
        let after = [group(
            2,
            "Reviewed checklist",
            &[("d-me", Some("me")), ("d-bo", Some("bo")), ("d-bo-2", Some("bo"))],
        )];
        let events = t.groups(&after, Timestamp(5_000));
        let kinds = records(&events);
        let joined = |who: &str, action| {
            kinds.iter().any(|k| {
                matches!(k, MomentKind::Governance(p)
                    if p.action == action && p.target.as_str() == who)
            })
        };
        assert!(joined("bo", GovernanceAction::Joined), "{kinds:?}");
        assert!(joined("ana", GovernanceAction::Removed), "{kinds:?}");
        assert!(kinds.iter().any(|k| matches!(k, MomentKind::GroupRename(p)
            if p.old_name == "Checklist" && p.new_name == "Reviewed checklist")));
        // Two devices of one new principal are one record.
        assert_eq!(kinds.len(), 3, "{kinds:?}");
        // The same listing again changes nothing.
        assert!(records(&t.groups(&after, Timestamp(9_000))).is_empty());
    }

    #[test]
    fn an_unnamed_member_and_a_second_device_report_no_person() {
        let mut t = translator();
        let events = t.groups(
            &[group(
                2,
                "Checklist",
                &[
                    ("d-me", Some("me")),
                    ("d-ana", Some("ana")),
                    ("d-ana-2", Some("ana")),
                    ("d-x", None),
                ],
            )],
            Timestamp(5_000),
        );
        assert!(records(&events).is_empty(), "no new principal, no new name");
    }
}
