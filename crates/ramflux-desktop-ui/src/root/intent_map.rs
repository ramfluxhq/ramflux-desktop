// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Maps a root-level UI event to the intent it requests.
//!
//! The target comes from the event's scope path (innermost context, moment,
//! surface and block) and from ids the event names. A UI event can never
//! carry human confirmation or a secret; both arrive by other paths.

use ramflux_desktop_core::{
    BlockId, BlockKind, GrantId, IntentKind, Moment, MomentKind, NodeAction, ObjectAction, Store,
    UiEvent, UiEventKind,
};

/// Maps a root-level UI event to its intent kind.
///
/// Returns `None` for view-local events and for events whose scope does not
/// identify a valid target. Validation beyond targeting is the policy's job.
pub fn intent_for(event: &UiEvent, store: &Store) -> Option<IntentKind> {
    context_intent(event)
        .or_else(|| moment_intent(event, store))
        .or_else(|| page_intent(event, store))
}

/// Intents addressed to the innermost context.
fn context_intent(event: &UiEvent) -> Option<IntentKind> {
    let context_id = event.context_id()?.clone();
    Some(match &event.kind {
        UiEventKind::SubmitMessage { text } => {
            IntentKind::SendMessage { context_id, text: text.clone() }
        }
        UiEventKind::AttachFile { path } => {
            IntentKind::AttachObject { context_id, path: path.clone() }
        }
        UiEventKind::TaskRequested { assignee, text } => {
            IntentKind::StartTask { context_id, assignee: assignee.clone(), text: text.clone() }
        }
        UiEventKind::PinToggled { pinned: true } => IntentKind::PinContext { context_id },
        UiEventKind::PinToggled { pinned: false } => IntentKind::UnpinContext { context_id },
        UiEventKind::MuteToggled { muted } => IntentKind::MuteContext { context_id, muted: *muted },
        UiEventKind::DisappearingChosen { seconds } => {
            IntentKind::SetDisappearing { context_id, seconds: *seconds }
        }
        UiEventKind::GovernanceRequested { op } => {
            IntentKind::Governance { context_id, op: op.clone() }
        }
        UiEventKind::RenameRequested { title } => {
            IntentKind::RenameContext { context_id, title: title.clone() }
        }
        _ => return None,
    })
}

/// Intents addressed to the innermost moment.
fn moment_intent(event: &UiEvent, store: &Store) -> Option<IntentKind> {
    let moment = event.moment_id().and_then(|id| store.moment(id))?;
    let moment_id = moment.id.clone();
    Some(match &event.kind {
        UiEventKind::ActionInvoked { action_id, values, surface_hash, version } => {
            let MomentKind::AgentSurface(surface) = &moment.kind else { return None };
            IntentKind::SurfaceAction {
                moment_id,
                surface_id: event.surface_id().unwrap_or(&surface.surface_id).clone(),
                action_id: action_id.clone(),
                values: values.clone(),
                // The version the user saw, never the store's current one.
                surface_hash: surface_hash.clone(),
                version: *version,
            }
        }
        UiEventKind::SurfaceUpdateRequested => {
            let MomentKind::AgentSurface(surface) = &moment.kind else { return None };
            IntentKind::RequestSurfaceUpdate {
                moment_id,
                surface_id: event.surface_id().unwrap_or(&surface.surface_id).clone(),
            }
        }
        UiEventKind::ApprovePressed => {
            IntentKind::Approve { grant_id: decision_grant(event, moment, store)? }
        }
        UiEventKind::DenyPressed => {
            IntentKind::Deny { grant_id: decision_grant(event, moment, store)? }
        }
        UiEventKind::RevealPressed => IntentKind::Reveal { moment_id },
        UiEventKind::ConflictResolved { resolution } => {
            IntentKind::ResolveConflict { moment_id, resolution: *resolution }
        }
        UiEventKind::CancelPressed => {
            let MomentKind::TaskProgress(payload) = &moment.kind else { return None };
            IntentKind::CancelTask { task_id: payload.task_id.clone() }
        }
        UiEventKind::ProposalAccepted => {
            let MomentKind::TaskProgress(payload) = &moment.kind else { return None };
            IntentKind::AcceptTaskProposal { task_id: payload.task_id.clone() }
        }
        UiEventKind::ContactAnswered { accept: true } => IntentKind::AcceptContact { moment_id },
        UiEventKind::ContactAnswered { accept: false } => IntentKind::RejectContact { moment_id },
        UiEventKind::HandoffAnswered { accept: true } => IntentKind::AcceptHandoff { moment_id },
        UiEventKind::HandoffAnswered { accept: false } => IntentKind::DeclineHandoff { moment_id },
        _ => return None,
    })
}

/// The grant an approve or deny press decides: the intervention's grant, an
/// approval block's grant, or the grant of the task step that needs the user.
fn decision_grant(event: &UiEvent, moment: &Moment, store: &Store) -> Option<GrantId> {
    match &moment.kind {
        MomentKind::Intervention(payload) => Some(payload.grant_id.clone()),
        MomentKind::AgentSurface(surface) => {
            let block_id = event.block_id()?;
            let block = surface.blocks.iter().find(|b| &b.id == block_id)?;
            if block.kind != BlockKind::Approval {
                return None;
            }
            block.data.get("grant_id").and_then(|v| v.as_str()).map(GrantId::new)
        }
        MomentKind::TaskProgress(payload) => {
            let task = store.task(&payload.task_id)?;
            let step = event
                .block_id()
                .and_then(step_index)
                .and_then(|ix| task.steps.get(ix))
                .or_else(|| task.needs_me_step().map(|(_, step)| step))?;
            step.grant_id().cloned()
        }
        _ => None,
    }
}

/// Block id of a task step row: `step-{index}`.
pub fn step_block_id(index: usize) -> BlockId {
    BlockId::new(format!("step-{index}"))
}

fn step_index(block: &BlockId) -> Option<usize> {
    block.as_str().strip_prefix("step-")?.parse().ok()
}

/// Intents named by id, from docks, settings, entry and object rows.
fn page_intent(event: &UiEvent, store: &Store) -> Option<IntentKind> {
    Some(match &event.kind {
        UiEventKind::StartConversationRequested { context_id, participant_id, text } => {
            IntentKind::StartConversation {
                context_id: context_id.clone(),
                participant_id: participant_id.clone(),
                text: text.clone(),
            }
        }
        UiEventKind::VerifySafetyPressed { participant_id } => {
            IntentKind::VerifySafetyNumber { participant_id: participant_id.clone() }
        }
        UiEventKind::AddContactRequested { participant_id } => {
            IntentKind::AddContact { participant_id: participant_id.clone() }
        }
        UiEventKind::AddContactCardRequested { card } => {
            IntentKind::AddContactCard { card: card.clone() }
        }
        UiEventKind::ShareContactCardRequested => IntentKind::ExportContactCard,
        UiEventKind::CreateGroupRequested { context_id, title, members } => {
            IntentKind::CreateGroup {
                context_id: context_id.clone(),
                title: title.clone(),
                members: members.clone(),
            }
        }
        UiEventKind::ContactActionPressed { participant_id, action } => {
            IntentKind::Contact { participant_id: participant_id.clone(), action: *action }
        }
        UiEventKind::ObjectActionPressed { object_id, action } => {
            let object_id = object_id.clone();
            match action {
                ObjectAction::Pull => IntentKind::PullObject { object_id },
                ObjectAction::Save { path } => {
                    IntentKind::SaveObject { object_id, path: path.clone() }
                }
                ObjectAction::Retry => IntentKind::RetryTransfer { object_id },
            }
        }
        UiEventKind::RevokeGrantPressed { grant_id } => {
            IntentKind::RevokeGrant { grant_id: grant_id.clone() }
        }
        UiEventKind::RevokeStandingPressed { approval_id } => {
            IntentKind::RevokeStandingApproval { approval_id: approval_id.clone() }
        }
        UiEventKind::AddNodeRequested { url } => IntentKind::AddNode { url: url.clone() },
        UiEventKind::ConfirmNodePressed { node_id } => IntentKind::ConfirmNode {
            node_id: node_id.clone(),
            pin_source: store.node(node_id)?.pin_source,
        },
        UiEventKind::NodeActionPressed { node_id, action } => {
            let node_id = node_id.clone();
            match action {
                NodeAction::Probe => IntentKind::ProbeNode { node_id },
                NodeAction::Remove => IntentKind::RemoveNode { node_id },
                NodeAction::SetDefault => IntentKind::SetDefaultNode { node_id },
                NodeAction::SetEnabled(enabled) => {
                    IntentKind::SetNodeEnabled { node_id, enabled: *enabled }
                }
            }
        }
        UiEventKind::LockPressed { account_id } => {
            IntentKind::LockAccount { account_id: account_id.clone() }
        }
        UiEventKind::RevokeDevicePressed { device_id } => {
            IntentKind::RevokeDevice { device_id: device_id.clone() }
        }
        UiEventKind::DeviceSyncRequested { direction, path } => {
            IntentKind::SyncDevices { direction: *direction, path: path.clone() }
        }
        UiEventKind::DiagnosticsExportRequested { path } => {
            IntentKind::ExportDiagnostics { path: path.clone() }
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use ramflux_desktop_core::{ContextId, MomentId, Scope};
    use ramflux_desktop_test::{ScenarioPlayer, load_fixture};

    use super::*;

    fn player(name: &str, ms: u64) -> ScenarioPlayer {
        let mut player = ScenarioPlayer::new(&load_fixture(name).expect("fixture"));
        player.advance(ms);
        player
    }

    fn at(context: &str, moment: &str, block: &str, kind: UiEventKind) -> UiEvent {
        UiEvent::new(Scope::Block(BlockId::new(block)), kind)
            .within(Scope::Moment(MomentId::new(moment)))
            .within(Scope::Context(ContextId::new(context)))
            .within(Scope::Workspace)
    }

    #[test]
    fn approvals_map_to_the_grant_of_their_moment_block_or_step() {
        let s03 = player("s03_intervention", 1_000);
        let intervention = at("c-ops", "m-approve", "decision", UiEventKind::ApprovePressed);
        assert!(matches!(
            intent_for(&intervention, s03.store()),
            Some(IntentKind::Approve { grant_id }) if grant_id == GrantId::new("g-rotate")
        ));
        let block = at("c-ops", "m-window", "b-notice", UiEventKind::DenyPressed);
        assert!(matches!(
            intent_for(&block, s03.store()),
            Some(IntentKind::Deny { grant_id }) if grant_id == GrantId::new("g-notice")
        ));
        let not_an_approval = at("c-ops", "m-window", "b-slot", UiEventKind::ApprovePressed);
        assert!(intent_for(&not_an_approval, s03.store()).is_none());
    }

    #[test]
    fn view_local_and_untargeted_events_map_to_nothing() {
        let s03 = player("s03_intervention", 1_000);
        let expand =
            at("c-ops", "m-approve", "details", UiEventKind::ExpandToggled { expanded: true });
        assert!(intent_for(&expand, s03.store()).is_none());
        let orphan =
            UiEvent::new(Scope::Workspace, UiEventKind::SubmitMessage { text: "x".into() });
        assert!(intent_for(&orphan, s03.store()).is_none());
        let wrong_kind = at("c-ops", "m-approve", "decision", UiEventKind::CancelPressed);
        assert!(intent_for(&wrong_kind, s03.store()).is_none());
    }

    #[test]
    fn confirm_node_takes_the_discovered_pin_source() {
        let s01 = player("s01_first_launch", 3_000);
        let event = UiEvent::new(
            Scope::Panel("entry".into()),
            UiEventKind::ConfirmNodePressed { node_id: "n-harbor".into() },
        )
        .within(Scope::Workspace);
        assert!(matches!(
            intent_for(&event, s01.store()),
            Some(IntentKind::ConfirmNode {
                pin_source: ramflux_desktop_core::PinSource::Invite,
                ..
            })
        ));
    }
}
