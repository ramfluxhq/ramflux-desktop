// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Headless screen adapters. Read plans use the existing source's snapshot path;
//! mutations pass through core policy and retain pending human confirmations.

use crate::plan::{
    self, Known, Planned,
    fetch::{Fetch, Paged},
};
use ramflux_desktop_core::{
    AccountId, Effect, Intent, IntentError, Store, Timestamp, handle_intent,
};

/// Complete directory presentation with source activity for unloaded histories.
/// Search covers metadata and cached bodies; this is not server full-text search.
///
/// # Errors
/// Returns a bounded-query error before normalization of oversized input.
pub fn conversation_page<'a>(
    store: &'a Store,
    directory: &crate::directory::Directory,
    query: &str,
    offset: usize,
    limit: usize,
) -> Result<
    ramflux_desktop_core::projection::conversations::ConversationPage<'a>,
    ramflux_desktop_core::projection::conversations::QueryError,
> {
    ramflux_desktop_core::projection::conversations::conversations_with_activity(
        store,
        query,
        offset,
        limit,
        |id| directory.conversations.get(id).and_then(|row| row.last_activity),
    )
}

/// Business area whose current facts a native screen needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Area {
    /// Complete conversation directory and participant names.
    Conversations,
    /// Contacts and their incoming/outgoing requests.
    Contacts,
    /// Pending strangers and request settings.
    Strangers,
    /// Groups, roles and eligible contacts.
    Groups,
    /// Owned Agent identities and runtime configuration.
    Agents,
    /// Account devices, including platform/origin facts when supported.
    Devices,
    /// Full backup/restore job state.
    Backup,
    /// Account, node and request preferences.
    Settings,
}

/// Contract work that must arrive before a complete screen can be wired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractGap {
    /// Agent runtime/tool/scope catalog and configuration operations.
    AgentConfiguration,
    /// Authenticated browser platform and exact origin in the device directory.
    WebDeviceFacts,
    /// Approving a new device with authenticated pairing review facts.
    DevicePairingApproval,
    /// Open-account visibility facts and mutation (not contact request policy).
    AccountPrivacy,
    /// Reply without acceptance, report and batch outcomes for strangers.
    StrangerActions,
}

/// Ordered reads and explicit unsupported parts. No fake business acceptance.
#[derive(Debug)]
#[non_exhaustive]
pub struct ReadPlan {
    /// Reads handled by the existing bridge and translator.
    pub reads: Vec<Fetch>,
    /// Missing backend contracts for this area.
    pub gaps: Vec<ContractGap>,
}

/// Plans authoritative reads; paged reads start at page one and follow source cursors.
#[must_use]
pub fn read_plan(area: Area, account: &AccountId) -> ReadPlan {
    let (reads, gaps) = match area {
        Area::Conversations => (
            vec![
                Fetch::first(Paged::Identities),
                Fetch::first(Paged::Conversations),
                Fetch::first(Paged::Groups),
            ],
            vec![],
        ),
        Area::Contacts => (vec![Fetch::ContactList, Fetch::ContactRequests], vec![]),
        Area::Strangers => (vec![Fetch::ContactRequests], vec![ContractGap::StrangerActions]),
        Area::Groups => (
            vec![Fetch::first(Paged::Groups), Fetch::first(Paged::Identities), Fetch::ContactList],
            vec![],
        ),
        Area::Agents => (
            vec![Fetch::AccountList, Fetch::first(Paged::Identities)],
            vec![ContractGap::AgentConfiguration],
        ),
        Area::Devices => (
            vec![Fetch::DeviceList],
            vec![ContractGap::WebDeviceFacts, ContractGap::DevicePairingApproval],
        ),
        Area::Backup => (vec![Fetch::BackupJobs], vec![]),
        Area::Settings => (
            vec![Fetch::AccountStatus(account.clone()), Fetch::NodeList, Fetch::ContactRequests],
            vec![ContractGap::AccountPrivacy],
        ),
    };
    ReadPlan { reads, gaps }
}

/// A policy-approved effect or a still-pending attended-human gesture.
#[derive(Debug)]
pub enum BusinessAction {
    /// Submit through the existing source, which owns retry and completion.
    Effect(Effect),
    /// Present the existing host confirmation flow; never send this as a request.
    Confirmation(Box<Intent>),
}

/// Applies the existing policy for all conversation/contact/group/Agent/device/
/// backup/settings intents. This adapter never supplies a confirmation itself.
///
/// # Errors
/// Returns the core policy's typed refusal, before any I/O or mutation.
pub fn actions(
    store: &Store,
    intent: Intent,
    now: Timestamp,
) -> Result<Vec<BusinessAction>, IntentError> {
    handle_intent(store, intent, now).map(|effects| {
        effects
            .into_iter()
            .map(|effect| match effect {
                Effect::RequireConfirmation { intent } => BusinessAction::Confirmation(intent),
                other => BusinessAction::Effect(other),
            })
            .collect()
    })
}

impl BusinessAction {
    /// Maps only an executable effect to the existing daemon planner. Pending
    /// confirmations are returned unchanged so a headless host cannot lose them.
    ///
    /// # Errors
    /// Returns the unchanged action when a human gesture is still required.
    pub fn into_plan(
        self,
        account: Option<&AccountId>,
        known: &impl Known,
    ) -> Result<Planned, Self> {
        match self {
            Self::Effect(effect) => Ok(plan::plan(account, effect, known)),
            pending => Err(pending),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ramflux_desktop_core::{
        ContextId, ContextKind, DomainEvent, EventEnvelope, IntentKind, NodeId, PinSource,
    };

    struct EmptyKnown;

    #[test]
    fn directory_activity_orders_rows_before_their_history_is_loaded() {
        let mut store = Store::new();
        let mut directory = crate::directory::Directory::default();
        for (seq, id, at) in [(1, "older", 5), (2, "newer", 10)] {
            let context = serde_json::from_value(
                serde_json::json!({"id":id,"kind":"Direct","title":id,"unread":3}),
            )
            .expect("test: context fixture");
            store
                .apply(&EventEnvelope::new(
                    seq,
                    Timestamp(0),
                    DomainEvent::ContextUpserted(context),
                ))
                .expect("test: context fact");
            directory.conversations.insert(
                ContextId::new(id),
                crate::directory::Conversation::from_row(
                    &serde_json::json!({"id":id,"kind":"direct","last_activity":at}),
                ),
            );
        }
        let page = conversation_page(&store, &directory, "", 0, 100)
            .expect("test: directory presentation");
        assert_eq!(
            page.rows.first().map(|r| r.context.id.as_str()),
            Some("newer"),
            "an unopened conversation must keep its authoritative recency"
        );
        assert_eq!(
            page.rows.first().and_then(|r| r.at),
            Some(Timestamp(10_000)),
            "directory activity must reach the row without loading a body"
        );
        assert_eq!(
            page.rows.first().map(|r| r.context.unread),
            Some(3),
            "reading the directory must not consume unread counts"
        );
    }
    impl Known for EmptyKnown {
        fn context_kind(&self, _: &ContextId) -> Option<ContextKind> {
            None
        }
        fn self_device(&self) -> Option<&str> {
            None
        }
        fn daemon_grant_id(&self, _: &str) -> Option<&str> {
            None
        }
    }

    #[test]
    fn every_area_has_real_reads_and_names_its_contract_gaps() {
        let account = AccountId::new("owner");
        for area in [
            Area::Conversations,
            Area::Contacts,
            Area::Strangers,
            Area::Groups,
            Area::Agents,
            Area::Devices,
            Area::Backup,
            Area::Settings,
        ] {
            let plan = read_plan(area, &account);
            assert!(!plan.reads.is_empty(), "{area:?} must load source facts");
            for read in plan.reads {
                let request = read.request(Some(account.as_str()));
                assert!(
                    request.operation_id.is_none(),
                    "{area:?} reads cannot reserve mutation receipts"
                );
                assert!(request.secrets.is_empty(), "{area:?} snapshots cannot carry secrets");
            }
        }
        assert_eq!(
            read_plan(Area::Agents, &account).gaps,
            vec![ContractGap::AgentConfiguration],
            "missing configuration cannot be represented as a successful identity read"
        );
        assert!(
            read_plan(Area::Devices, &account).gaps.contains(&ContractGap::DevicePairingApproval),
            "joining with a code is not authority to approve another device"
        );
        assert!(
            read_plan(Area::Settings, &account).gaps.contains(&ContractGap::AccountPrivacy),
            "request acceptance cannot masquerade as open-account visibility"
        );
        assert!(
            read_plan(Area::Conversations, &account).gaps.is_empty(),
            "directory reads have a current daemon counterpart"
        );
    }

    #[test]
    fn pending_confirmation_is_preserved_and_cannot_become_a_daemon_call() {
        let mut store = Store::new();
        let node=serde_json::from_value(serde_json::json!({"id":"node","url":"https://node.example","pin_source":"Manual","confirmed":false})).expect("test: node fixture");
        store
            .apply(&EventEnvelope::new(1, Timestamp(1), DomainEvent::NodeUpserted(node)))
            .expect("test: source node fact");
        let intent = Intent::human(IntentKind::ConfirmNode {
            node_id: NodeId::new("node"),
            pin_source: PinSource::Manual,
        });
        let mut prepared =
            actions(&store, intent, Timestamp(2)).expect("test: policy requests a confirmation");
        let pending = prepared.pop().expect("test: one confirmation action");
        assert!(
            matches!(pending.into_plan(None, &EmptyKnown), Err(BusinessAction::Confirmation(_))),
            "a missing human gesture must never be collapsed into a refusal or sent request"
        );
    }
}
