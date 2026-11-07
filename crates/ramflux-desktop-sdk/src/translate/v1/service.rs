// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! A v1 service event: membership, group rename and key change rows. They
//! share the conversation stream with messages and order by the same key.

use ramflux_desktop_core::{
    DomainEvent, GovernanceAction, GovernancePayload, GroupRenamePayload, MomentKind,
    SafetyChangeKind, SafetyChangePayload, SystemLevel, SystemPayload,
};
use ramflux_protocol::message::{EventIdentity, Service, ServiceAction};

use super::{context_of, moment_id, moment_of, unsupported};
use crate::translate::Translator;

pub(super) fn project(
    translator: &mut Translator,
    identity: &EventIdentity,
    service: &Service,
) -> Vec<DomainEvent> {
    let context = context_of(identity);
    let device = identity.author.as_ref().map_or("", |a| a.device_id.as_str());
    let origin = translator.origin(device);
    let subjects: Vec<_> =
        service.subjects.iter().map(|s| translator.origin(&s.device_id)).collect();
    let kinds: Vec<MomentKind> = match ServiceAction::try_from(service.action) {
        Ok(ServiceAction::ServiceMemberAdded) => subjects
            .into_iter()
            .map(|target| governance(GovernanceAction::Joined, target))
            .collect(),
        Ok(ServiceAction::ServiceMemberRemoved) => subjects
            .into_iter()
            .map(|target| governance(GovernanceAction::Removed, target))
            .collect(),
        Ok(ServiceAction::ServiceNameChanged) => {
            vec![MomentKind::GroupRename(GroupRenamePayload {
                old_name: translator.context_title(&context).unwrap_or_default(),
                new_name: service.text.clone().unwrap_or_default(),
            })]
        }
        Ok(ServiceAction::ServiceKeyChanged) => subjects
            .into_iter()
            .map(|participant_id| {
                MomentKind::SafetyChange(SafetyChangePayload {
                    participant_id,
                    change: SafetyChangeKind::KeyChanged,
                })
            })
            .collect(),
        // The schema carries no pin/unpin flag: a change that names its
        // target message is a pin, one that names none is an unpin.
        Ok(ServiceAction::ServicePinChanged) => {
            let key = if service.target.is_some() { "pinned" } else { "unpinned" };
            let args = serde_json::json!({ "actor": translator.display_of(&origin) });
            vec![MomentKind::System(SystemPayload {
                level: SystemLevel::Info,
                text: format!("backend.service.{key} {args}"),
            })]
        }
        Ok(ServiceAction::ServiceUnspecified) | Err(_) => {
            vec![unsupported("unknown_service_action", "unknown_service_action")]
        }
    };
    kinds
        .into_iter()
        .enumerate()
        .filter_map(|(index, kind)| {
            let mut id = moment_id(&context, &identity.id);
            if index > 0 {
                id = ramflux_desktop_core::MomentId::new(format!("{id}#{index}"));
            }
            translator.report_moment(moment_of(identity, id, origin.as_str(), kind))
        })
        .collect()
}

fn governance(action: GovernanceAction, target: ramflux_desktop_core::ParticipantId) -> MomentKind {
    MomentKind::Governance(GovernancePayload { action, target, role: None, unattributed: false })
}
