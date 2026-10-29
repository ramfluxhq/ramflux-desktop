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
//! device that was listed before without a principal and is named now is
//! the same member being identified, not a join; likewise a device that
//! loses its name while still listed is not a removal. A record is reported
//! once: its id carries the group's epoch (or the name epoch), so replays
//! and repeated listings do not repeat it.
//!
//! The signed state names the group's writer, not who asked for a change,
//! so the roster records are unattributed: they say who joined or was
//! removed, never who did it.

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

    /// The devices in the roster, named or not.
    fn devices(&self) -> BTreeSet<&str> {
        self.members.iter().map(|m| m.member_id.as_str()).collect()
    }

    /// The devices of `principal` in the roster.
    fn devices_of<'a>(&'a self, principal: &'a str) -> impl Iterator<Item = &'a str> {
        self.members
            .iter()
            .filter(move |m| m.principal_id.as_deref() == Some(principal))
            .map(|m| m.member_id.as_str())
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
            let (was_devices, is_devices) = (previous.devices(), group.devices());
            // A principal that appears with a device already listed before
            // (then unnamed) was identified, not added; one that disappears
            // while its device is still listed (now unnamed) was not removed.
            let joined = is
                .difference(&was)
                .filter(|p| !group.devices_of(p).any(|device| was_devices.contains(device)));
            let removed = was
                .difference(&is)
                .filter(|p| !previous.devices_of(p).any(|device| is_devices.contains(device)));
            for (action, who) in [
                (GovernanceAction::Joined, joined.collect::<Vec<_>>()),
                (GovernanceAction::Removed, removed.collect()),
            ] {
                for principal in who {
                    let target = ParticipantId::new(*principal);
                    let id = format!("group-sync:{context}:{action:?}:{principal}:{stamp}");
                    // The record names no author; the origin is this
                    // account, as for the rename line.
                    let origin = self.directory.self_principal.clone().unwrap_or_default();
                    events.extend(self.report_moment(Moment {
                        id: MomentId::new(id),
                        context_id: context.clone(),
                        origin: ParticipantId::new(origin),
                        at: now,
                        kind: MomentKind::Governance(GovernancePayload {
                            action,
                            target,
                            role: None,
                            unattributed: true,
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

#[cfg(test)]
mod identification_tests {
    use serde_json::json;

    use super::*;
    use crate::translate::Scope;

    fn group(epoch: u64, members: &[(&str, Option<&str>)]) -> serde_json::Value {
        let routes: Vec<_> = members
            .iter()
            .map(|(device, principal)| {
                json!({ "member_id": device, "principal_id": principal,
                        "delivery_target_known": true })
            })
            .collect();
        json!({ "group_id": "g-1", "group_epoch": epoch, "name": "G",
                "name_epoch": 1, "member_routes": routes })
    }

    fn translator(members: &[(&str, Option<&str>)]) -> Translator {
        let mut t = Translator::new(Scope::Daemon, "rfd.sock");
        let rows = [json!({ "id": "thread", "kind": "group", "group_id": "g-1", "title": "T" })];
        t.groups(&[group(1, members)], Timestamp(0));
        t.conversations(&rows);
        t
    }

    fn governance(events: &[DomainEvent]) -> Vec<GovernancePayload> {
        events
            .iter()
            .filter_map(|e| match e {
                DomainEvent::MomentAdded(m) => match &m.kind {
                    MomentKind::Governance(p) => Some(p.clone()),
                    _ => None,
                },
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_device_named_later_is_identified_not_joined() {
        let mut t = translator(&[("d-me", Some("me")), ("d-x", None)]);
        let events =
            t.groups(&[group(2, &[("d-me", Some("me")), ("d-x", Some("xia"))])], Timestamp(5));
        assert!(governance(&events).is_empty(), "{events:?}");
        // A device that is new, and named, is a real join.
        let events = t.groups(
            &[group(3, &[("d-me", Some("me")), ("d-x", Some("xia")), ("d-y", Some("yan"))])],
            Timestamp(6),
        );
        let joined = governance(&events);
        assert_eq!(joined.len(), 1, "{joined:?}");
        assert_eq!(joined[0].target.as_str(), "yan");
        assert_eq!(joined[0].action, GovernanceAction::Joined);
    }

    #[test]
    fn a_device_that_loses_its_name_is_not_removed_and_a_removal_is_unattributed() {
        let mut t = translator(&[("d-me", Some("me")), ("d-x", Some("xia")), ("d-y", Some("yan"))]);
        let events = t.groups(
            &[group(2, &[("d-me", Some("me")), ("d-x", None), ("d-y", Some("yan"))])],
            Timestamp(5),
        );
        assert!(governance(&events).is_empty(), "{events:?}");
        let events = t.groups(&[group(3, &[("d-me", Some("me")), ("d-x", None)])], Timestamp(6));
        let removed = governance(&events);
        assert_eq!(removed.len(), 1, "{removed:?}");
        assert_eq!(removed[0].target.as_str(), "yan");
        assert_eq!(removed[0].action, GovernanceAction::Removed);
        assert!(removed[0].unattributed, "the signed state does not say who removed them");
    }
}
