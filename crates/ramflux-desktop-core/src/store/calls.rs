// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Voice calls and their timeline records.
//!
//! A call update is the whole call with a rising version. It is validated
//! in full (known conversation, rising version, same conversation, peer,
//! caller and direction, a legal state transition) before anything
//! changes. The call's record in its conversation (one `Call` moment per
//! call, id `call:<call id>`, at the time the call was placed) is added with
//! the first update and follows the call's state from then on: a ringing
//! incoming call needs the user, a call under way is in progress, an ended
//! one is done.

use super::Store;
use super::changes::{ApplyError, Changes};
use crate::ids::Timestamp;
use crate::model::{Call, CallPayload, CallState, Moment, MomentKind};

impl Store {
    pub(super) fn upsert_call(
        &mut self,
        at: Timestamp,
        call: &Call,
        changes: &mut Changes,
    ) -> Result<(), ApplyError> {
        if !self.contexts.contains_key(&call.context_id) {
            return Err(ApplyError::UnknownContext(call.context_id.clone()));
        }
        if let Some(current) = self.calls.get(&call.id) {
            if call.version <= current.version {
                return Err(ApplyError::StaleCallVersion {
                    call: call.id.clone(),
                    current: current.version,
                    found: call.version,
                });
            }
            if (&call.context_id, &call.peer, &call.caller, call.direction)
                != (&current.context_id, &current.peer, &current.caller, current.direction)
            {
                return Err(ApplyError::CallIdentityChanged(call.id.clone()));
            }
            if call.state != current.state && !CallState::can_transition(current.state, call.state)
            {
                return Err(ApplyError::IllegalCallTransition {
                    call: call.id.clone(),
                    from: current.state,
                    to: call.state,
                });
            }
        }
        let moment_id = Call::moment_id(&call.id);
        let record = self.moments.get(&moment_id);
        if record.is_some_and(|m| !matches!(&m.kind, MomentKind::Call(p) if p.call_id == call.id)) {
            return Err(ApplyError::DuplicateMoment(moment_id));
        }
        let exists = record.is_some();
        changes.calls.insert(call.id.clone());
        self.calls.insert(call.id.clone(), call.clone());
        if exists {
            self.follow_task(at, &moment_id, call.state.moment_lifecycle(), changes);
            changes.moments.insert(moment_id);
        } else {
            let moment = Moment {
                id: moment_id,
                context_id: call.context_id.clone(),
                origin: call.caller.clone(),
                at: call.started_at,
                kind: MomentKind::Call(CallPayload { call_id: call.id.clone() }),
                lifecycle: call.state.moment_lifecycle(),
            };
            // Validated above (known context, the id is free): cannot fail.
            self.add_moment(at, &moment, changes)?;
        }
        Ok(())
    }
}
