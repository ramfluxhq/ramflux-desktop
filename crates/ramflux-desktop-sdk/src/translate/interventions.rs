// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Approvals shown in the conversation they came from.
//!
//! The daemon keeps the `conversation_id` of the MCP request that asked for
//! an approval. Such an approval becomes an intervention moment in that
//! conversation, waiting on the user (so it is in "needs me"), next to the
//! grant in the approvals dock. The decision closes it: approved is done,
//! denied is dismissed, a failed or expired grant is failed. An approval
//! without a conversation stays in the approvals dock only; a context is
//! never guessed. One whose conversation is not listed yet is placed when
//! it is.

use ramflux_desktop_core::{
    ContextId, DomainEvent, Grant, GrantId, GrantState, InterventionPayload, Lifecycle, Moment,
    MomentId, MomentKind, ParticipantId, Timestamp,
};
use serde_json::Value;

use super::Translator;
use crate::directory::field;

/// Where an approval is shown.
#[derive(Debug, Clone)]
pub(crate) struct Placement {
    context: ContextId,
    moment: Moment,
    shown: bool,
}

/// The moment id of an approval's intervention.
pub fn moment_id(grant: &GrantId) -> MomentId {
    MomentId::new(format!("approval:{grant}"))
}

impl Translator {
    /// Places a newly requested grant in its conversation, when it has one.
    pub(super) fn intervene(
        &mut self,
        grant: &Grant,
        record: &Value,
        now: Timestamp,
    ) -> Vec<DomainEvent> {
        let Some(context) = field(record, "conversation_id").map(ContextId::new) else {
            return Vec::new();
        };
        let moment = Moment {
            id: moment_id(&grant.id),
            context_id: context.clone(),
            // The MCP server that asked, as the daemon names it.
            origin: ParticipantId::new(field(record, "server_id").unwrap_or_default()),
            at: now,
            kind: MomentKind::Intervention(InterventionPayload {
                grant_id: grant.id.clone(),
                risk: grant.risk,
                summary: grant.scope.clone(),
            }),
            lifecycle: Lifecycle::NeedsMe,
        };
        self.interventions.insert(grant.id.clone(), Placement { context, moment, shown: false });
        self.place_interventions()
    }

    /// Shows the interventions whose conversation is listed now.
    pub(super) fn place_interventions(&mut self) -> Vec<DomainEvent> {
        let mut events = Vec::new();
        for placement in self.interventions.values_mut() {
            let listed = self.contexts.contains_key(&placement.context);
            if placement.shown && !listed {
                placement.shown = false;
            } else if !placement.shown && listed {
                placement.shown = true;
                events.push(DomainEvent::MomentAdded(Box::new(placement.moment.clone())));
            }
        }
        events
    }

    /// The decision on a grant closes its intervention.
    pub(super) fn intervention_closed(
        &mut self,
        grant: &GrantId,
        state: GrantState,
    ) -> Vec<DomainEvent> {
        let lifecycle = match state {
            GrantState::Approved => Lifecycle::Done,
            GrantState::Denied => Lifecycle::Dismissed,
            GrantState::Expired | GrantState::Revoked => Lifecycle::Failed,
            GrantState::Pending => return Vec::new(),
        };
        self.close(grant, lifecycle)
    }

    /// A failed grant fails its intervention.
    pub(super) fn intervention_failed(&mut self, grant: &GrantId) -> Vec<DomainEvent> {
        self.close(grant, Lifecycle::Failed)
    }

    fn close(&mut self, grant: &GrantId, lifecycle: Lifecycle) -> Vec<DomainEvent> {
        let Some(placement) = self.interventions.get_mut(grant) else { return Vec::new() };
        if placement.moment.lifecycle != Lifecycle::NeedsMe {
            return Vec::new();
        }
        placement.moment.lifecycle = lifecycle;
        if !placement.shown {
            return Vec::new();
        }
        vec![DomainEvent::MomentUpdated {
            moment_id: placement.moment.id.clone(),
            lifecycle: Some(lifecycle),
            kind: None,
        }]
    }
}
