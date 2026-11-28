// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Maps a root-level UI event to the intent it requests.
//!
//! The target comes from the event's scope path (innermost context, moment,
//! surface and block) and from ids the event names. A UI event can never
//! carry human confirmation or a secret; both arrive by other paths.

use ramflux_desktop_core::{
    BlockId, DeleteScope, GrantId, IntentKind, Moment, MomentKind, NodeAction, ObjectAction, Store,
    UiEvent, UiEventKind,
};

/// Maps a root-level UI event to its intent kind.
///
/// Returns `None` for view-local events and for events whose scope does not
/// identify a valid target. Validation beyond targeting is the policy's job.
pub fn intent_for(event: &UiEvent, store: &Store) -> Option<IntentKind> {
    mls_group_intent(event, store)
        .or_else(|| context_intent(event))
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
        UiEventKind::CallStartRequested => IntentKind::StartCall { context_id },
        UiEventKind::SubmitReply { target, text } => {
            IntentKind::ReplyMessage { moment_id: target.clone(), text: text.clone() }
        }
        UiEventKind::SubmitEdit { target, text } => {
            IntentKind::EditMessage { moment_id: target.clone(), text: text.clone() }
        }
        UiEventKind::LeaveGroupPressed => IntentKind::LeaveGroup { context_id },
        _ => return None,
    })
}

/// Intents addressed to the innermost moment.
fn moment_intent(event: &UiEvent, store: &Store) -> Option<IntentKind> {
    let moment = event.moment_id().and_then(|id| store.moment(id))?;
    let moment_id = moment.id.clone();
    Some(match &event.kind {
        UiEventKind::RetryMessagePressed => IntentKind::RetryMessage { moment_id },
        UiEventKind::DeleteForMePressed => {
            IntentKind::DeleteMessage { moment_id, scope: DeleteScope::Local }
        }
        UiEventKind::DeleteForEveryonePressed => {
            IntentKind::DeleteMessage { moment_id, scope: DeleteScope::Everyone }
        }
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
        UiEventKind::CardInteracted { key, event: input, seen } => {
            let MomentKind::AgentSurface(surface) = &moment.kind else { return None };
            IntentKind::CardInteract {
                moment_id,
                surface_id: event.surface_id().unwrap_or(&surface.surface_id).clone(),
                key: key.clone(),
                event: input.clone(),
                seen: seen.clone(),
            }
        }
        UiEventKind::CardSlotAnswered { slot_id, outcome, seen } => {
            let MomentKind::AgentSurface(surface) = &moment.kind else { return None };
            IntentKind::CardSlotAnswer {
                moment_id,
                surface_id: event.surface_id().unwrap_or(&surface.surface_id).clone(),
                slot_id: slot_id.clone(),
                outcome: *outcome,
                seen: seen.clone(),
            }
        }
        UiEventKind::CardSubmitRetried => {
            let MomentKind::AgentSurface(surface) = &moment.kind else { return None };
            IntentKind::CardSubmit {
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
        UiEventKind::RetryDecisionPressed => {
            let MomentKind::TaskProgress(payload) = &moment.kind else { return None };
            IntentKind::RetryTaskDecision { task_id: payload.task_id.clone() }
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
        // A card's approval is answered through its slot (a verified intent,
        // not a grant decision here).
        MomentKind::AgentSurface(_) => None,
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
        UiEventKind::AcceptIdentityPressed { participant_id, new_anchor } => {
            IntentKind::AcceptIdentityChange {
                participant_id: participant_id.clone(),
                new_anchor: new_anchor.clone(),
            }
        }
        UiEventKind::AddContactRequested { participant_id } => {
            IntentKind::AddContact { participant_id: participant_id.clone() }
        }
        UiEventKind::AddContactCardRequested { card, replace_provisional_root } => {
            IntentKind::AddContactCard {
                card: card.clone(),
                replace_provisional_root: replace_provisional_root.clone(),
            }
        }
        UiEventKind::ShareContactCardRequested => IntentKind::ExportContactCard,
        UiEventKind::AddContactByUsernameRequested { name } => {
            IntentKind::AddContactByUsername { name: name.clone() }
        }
        UiEventKind::ResolveUsernameRequested { name } => {
            IntentKind::ResolveUsername { name: name.clone() }
        }
        UiEventKind::UsernameStatusRefreshRequested { operation_id } => {
            IntentKind::UsernameStatusRefresh { operation_id: operation_id.clone() }
        }
        UiEventKind::GroupInvitationAnswered { group_id, accept } => {
            IntentKind::AnswerGroupInvitation { group_id: group_id.clone(), accept: *accept }
        }
        UiEventKind::CallAnswered { call_id, accept: true } => {
            IntentKind::AnswerCall { call_id: call_id.clone() }
        }
        UiEventKind::CallAnswered { call_id, accept: false } => {
            IntentKind::RejectCall { call_id: call_id.clone() }
        }
        UiEventKind::CallHangUpPressed { call_id } => {
            IntentKind::HangUpCall { call_id: call_id.clone() }
        }
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
        UiEventKind::ContactRequestAnswered { request_id, answer } => {
            IntentKind::AnswerContactRequest { request_id: request_id.clone(), answer: *answer }
        }
        UiEventKind::ContactRequestPolicySwitched { auto_accept } => {
            IntentKind::SetContactRequestPolicy {
                policy: if *auto_accept {
                    ramflux_desktop_core::RequestPolicy::AutoAccept
                } else {
                    ramflux_desktop_core::RequestPolicy::Manual
                },
            }
        }
        UiEventKind::ClearRequestFloorsPressed => IntentKind::ClearRequestFloors,
        UiEventKind::ObjectActionPressed { object_id, action } => {
            let object_id = object_id.clone();
            match action {
                ObjectAction::Pull => IntentKind::PullObject { object_id },
                ObjectAction::Save { path } => {
                    IntentKind::SaveObject { object_id, path: path.clone() }
                }
                ObjectAction::Retry => IntentKind::RetryTransfer { object_id },
                ObjectAction::Cancel => IntentKind::CancelTransfer { object_id },
            }
        }
        UiEventKind::RevokeGrantPressed { grant_id } => {
            IntentKind::RevokeGrant { grant_id: grant_id.clone() }
        }
        UiEventKind::RevokeStandingPressed { approval_id } => {
            IntentKind::RevokeStandingApproval { approval_id: approval_id.clone() }
        }
        UiEventKind::AgentApprovalPressed { owner_id, approval_id, approved } => {
            IntentKind::AgentApprovalDecide {
                owner_id: owner_id.clone(),
                approval_id: approval_id.clone(),
                approved: *approved,
            }
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
        UiEventKind::AppLockConfigureRequested { account_id, enabled, timeout_ms } => {
            IntentKind::ConfigureAppLock {
                account_id: account_id.clone(),
                enabled: *enabled,
                timeout_ms: *timeout_ms,
            }
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

/// The ids of own sends made by this process: a time and a counter, so two
/// quick sends never share a `local_id`.
fn new_local_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    format!("l-{now}-{}", COUNTER.fetch_add(1, Ordering::Relaxed))
}

/// The group an MLS context names.
fn mls_group_of(event: &UiEvent) -> Option<String> {
    ramflux_desktop_core::mls_group::group_of(event.context_id()?).map(str::to_owned)
}

/// The post a moment of an MLS group shows: posted (`mls:<group>:<log_seq>`)
/// or own and not in the log (`mls:<group>:p:<post_id>`).
pub fn mls_post_of(store: &Store, moment: &ramflux_desktop_core::MomentId) -> Option<String> {
    let rest = moment.as_str().strip_prefix(ramflux_desktop_core::mls_group::CONTEXT_PREFIX)?;
    let (group, tail) = rest.split_once(':')?;
    let timeline = store.mls_timeline(group)?;
    if let Some(post) = tail.strip_prefix("p:") {
        return Some(post.to_owned());
    }
    let log_seq: u64 = tail.split(':').next()?.parse().ok()?;
    timeline.posted.get(&log_seq).map(|p| p.post_id.clone())
}

/// Whether a moment of an MLS group is a post that is in the log
/// (`mls:<group>:<log_seq>`), not an own one that is still on its way.
pub fn mls_posted(moment: &ramflux_desktop_core::MomentId) -> bool {
    moment
        .as_str()
        .strip_prefix(ramflux_desktop_core::mls_group::CONTEXT_PREFIX)
        .and_then(|rest| rest.split_once(':'))
        .is_some_and(|(_, tail)| !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit()))
}

/// The intents of the MLS groups: their own events, and what the composer
/// submits in an MLS group context (a post, a reply, a Retry).
fn mls_group_intent(event: &UiEvent, store: &Store) -> Option<IntentKind> {
    use ramflux_desktop_core::mls_group::{self, HistoryDirection};
    Some(match &event.kind {
        UiEventKind::MlsListRequested { cursor } => {
            IntentKind::MlsGroupList { cursor: cursor.clone() }
        }
        UiEventKind::MlsInfoRequested { group_id } => {
            IntentKind::MlsGroupInfo { group_id: group_id.clone() }
        }
        UiEventKind::MlsMembersRequested { group_id, cursor } => {
            IntentKind::MlsGroupMembers { group_id: group_id.clone(), cursor: cursor.clone() }
        }
        UiEventKind::MlsHistoryRequested { group_id, direction } => IntentKind::MlsGroupHistory {
            group_id: group_id.clone(),
            direction: match direction {
                HistoryDirection::Newest => HistoryDirection::Newest,
                HistoryDirection::Before(ls) => HistoryDirection::Before(*ls),
                HistoryDirection::After(ls) => HistoryDirection::After(*ls),
            },
        },
        UiEventKind::MlsRefreshPressed { group_id } => {
            IntentKind::MlsGroupRefresh { group_id: group_id.clone() }
        }
        UiEventKind::MlsMarkReadRequested { group_id, log_seq } => {
            IntentKind::MlsGroupMarkRead { group_id: group_id.clone(), log_seq: *log_seq }
        }
        UiEventKind::MlsCreatePressed { name, approval } => {
            IntentKind::MlsGroupCreate { name: name.clone(), approval: *approval }
        }
        UiEventKind::MlsInviteCreatePressed { group_id, max_uses, ttl_s } => {
            IntentKind::MlsInviteCreate {
                group_id: group_id.clone(),
                max_uses: *max_uses,
                ttl_s: *ttl_s,
            }
        }
        UiEventKind::MlsInviteListRequested { group_id } => {
            IntentKind::MlsInviteList { group_id: group_id.clone() }
        }
        UiEventKind::MlsInviteRevokePressed { group_id, invite_id } => {
            IntentKind::MlsInviteRevoke { group_id: group_id.clone(), invite_id: invite_id.clone() }
        }
        UiEventKind::MlsJoinPressed { link, attempt } => {
            IntentKind::MlsGroupJoin { link: link.clone(), attempt: *attempt }
        }
        UiEventKind::MlsJoinPendingRequested => IntentKind::MlsJoinPending,
        UiEventKind::MlsDeletePressed { group_id, post_id } => {
            IntentKind::MlsGroupDelete { group_id: group_id.clone(), post_id: post_id.clone() }
        }
        UiEventKind::MlsLeavePressed { group_id } => {
            IntentKind::MlsGroupLeave { group_id: group_id.clone() }
        }
        // The admin and edge states (M3): one intent per confirmed press. The
        // epoch and the action id are stamped by the policy.
        UiEventKind::MlsRequestsRequested { group_id, cursor } => {
            IntentKind::MlsRequestsList { group_id: group_id.clone(), cursor: cursor.clone() }
        }
        UiEventKind::MlsApprovePressed { group_id, request_id } => IntentKind::MlsRequestApprove {
            group_id: group_id.clone(),
            request_id: request_id.clone(),
        },
        UiEventKind::MlsDeclinePressed { group_id, request_id } => IntentKind::MlsRequestDecline {
            group_id: group_id.clone(),
            request_id: request_id.clone(),
        },
        UiEventKind::MlsRolePressed { group_id, anchor, admin } => IntentKind::MlsRoleSet {
            group_id: group_id.clone(),
            anchor: anchor.clone(),
            admin: *admin,
        },
        UiEventKind::MlsRemovePressed { group_id, anchor } => {
            IntentKind::MlsMemberRemove { group_id: group_id.clone(), anchor: anchor.clone() }
        }
        UiEventKind::MlsBanPressed { group_id, anchor } => {
            IntentKind::MlsMemberBan { group_id: group_id.clone(), anchor: anchor.clone() }
        }
        UiEventKind::MlsSettingsPressed { group_id, field } => {
            IntentKind::MlsSettingsSet { group_id: group_id.clone(), field: field.clone() }
        }
        UiEventKind::MlsPushPressed { group_id, push } => {
            IntentKind::MlsPushSet { group_id: group_id.clone(), push: push.clone() }
        }
        UiEventKind::MlsSkipPressed { group_id } => {
            IntentKind::MlsCatchUpSkip { group_id: group_id.clone() }
        }
        UiEventKind::MlsVoidPressed { group_id, log_seq } => {
            IntentKind::MlsVoid { group_id: group_id.clone(), log_seq: *log_seq }
        }
        UiEventKind::MlsRetryPressed { context, local_id } => {
            let group_id = mls_group::group_of(context)?.to_owned();
            let item = store
                .mls_not_sent(&group_id)
                .into_iter()
                .find(|i| &i.local_id == local_id || i.post_id.as_deref() == Some(local_id))?;
            IntentKind::MlsGroupSend {
                group_id,
                text: item.text.clone(),
                reply_to: item.reply_to.clone(),
                mentions: item.mentions.clone(),
                local_id: new_local_id(),
                retry_of: Some(item.local_id.clone()),
            }
        }
        // The composer of an MLS group posts text, with the mentions the
        // user picked; a reply names the post it answers.
        UiEventKind::SubmitMessage { text } => {
            let group_id = mls_group_of(event)?;
            IntentKind::MlsGroupSend {
                group_id: group_id.clone(),
                text: text.clone(),
                reply_to: None,
                mentions: store
                    .mls_mentions(&group_id)
                    .into_iter()
                    .map(|(anchor, _)| anchor)
                    .collect(),
                local_id: new_local_id(),
                retry_of: None,
            }
        }
        UiEventKind::SubmitReply { target, text } => {
            let group_id = mls_group_of(event)?;
            IntentKind::MlsGroupSend {
                group_id: group_id.clone(),
                text: text.clone(),
                reply_to: mls_post_of(store, target),
                mentions: store
                    .mls_mentions(&group_id)
                    .into_iter()
                    .map(|(anchor, _)| anchor)
                    .collect(),
                local_id: new_local_id(),
                retry_of: None,
            }
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
    fn approvals_map_to_the_grant_of_their_intervention_or_step() {
        let s03 = player("s03_intervention", 1_000);
        let intervention = at("c-ops", "m-approve", "decision", UiEventKind::ApprovePressed);
        assert!(matches!(
            intent_for(&intervention, s03.store()),
            Some(IntentKind::Approve { grant_id }) if grant_id == GrantId::new("g-rotate")
        ));
        // A card's approval is answered through its slot, never as a grant.
        let card = at("c-ops", "m-window", "b-notice", UiEventKind::DenyPressed);
        assert!(intent_for(&card, s03.store()).is_none());
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
    fn mapping_a_group_post_reads_the_picked_mentions_and_keeps_them() {
        let s03 = player("s03_intervention", 1_000);
        let context = ramflux_desktop_core::mls_group::context_id("g-1");
        s03.store().mls_toggle_mention("g-1", "anc-1", "Ann");
        let event = UiEvent::new(
            Scope::Panel("composer".into()),
            UiEventKind::SubmitMessage { text: "hi".into() },
        )
        .within(Scope::Context(context))
        .within(Scope::Workspace);
        for _ in 0..2 {
            // A send the policy then refuses must not have used the picks up.
            assert!(matches!(
                intent_for(&event, s03.store()),
                Some(IntentKind::MlsGroupSend { mentions, .. }) if mentions == ["anc-1"]
            ));
        }
        assert_eq!(s03.store().mls_mentions("g-1").len(), 1);
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
