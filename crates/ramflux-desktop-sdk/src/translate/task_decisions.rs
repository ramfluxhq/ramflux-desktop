// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! A choice or an input a task step asks for, answered in the assignee's
//! surface, then resolved on the task.
//!
//! The step names the assignee's surface; the user acts on that surface
//! as on any other (`surface.action`), with values that say
//! `decision: "resume"` or `"deny"`. Such an action carries the step it
//! answers inside its signed bytes (`taskBinding {task_id, step_id}`), and
//! the author's creator receipt embeds that same action. A read of the
//! surface (`surface.get` `actions[]`) lists this account's bound actions,
//! the requester's own signed outgoing ones first (`receipt_verified:
//! false`, kept by the daemon across a restart), then with the author's
//! receipt. Each such row resolves its step with `task.resolve`,
//! `decision_id` being the row's `action_receipt_id` (the delivery receipt
//! id, the same before and after the receipt arrives; not the action
//! name), as long as the step still waits on that surface and no
//! resolution of it is pending. The step is named by the signed binding,
//! never by which step happens to be open, so a restarted desktop resolves
//! it from the read alone. Each receipt resolves at most one step, once.

use ramflux_desktop_core::{Actor, NeedsMe, StepState, TaskId, methods};
use ramflux_protocol::A2uiSurfaceAction;
use serde_json::{Value, json};

use super::{Translator, list, text};

/// A surface whose action answers a task step: read on each change until
/// the action's row resolves the step.
#[derive(Debug, Clone)]
pub(crate) struct SurfaceDecision {
    /// The surface (`surface_ref`).
    surface: String,
}

impl Translator {
    /// The task step an action on `reference` answers: the daemon's task id
    /// and the step id of a task this account asked for whose open step
    /// waits on that surface.
    pub(crate) fn surface_step(&self, reference: &str) -> Option<(String, String)> {
        self.tasks.values().find_map(|known| {
            if known.shown.requester != Actor::Human {
                return None;
            }
            let step = known.shown.steps.iter().find(|s| {
                s.state == StepState::NeedsMe
                    && s.needs_me.as_ref().and_then(NeedsMe::surface).map(|s| s.as_str())
                        == Some(reference)
            })?;
            Some((known.target.task_id.clone(), step.id.to_string()))
        })
    }

    /// After a surface action the daemon accepted: if it answers a task
    /// step, the surface is read on each change until the step resolves.
    pub(super) fn note_surface_decision(&mut self, method: &str, body: &Value, answer: &Value) {
        if method != methods::SURFACE_ACTION || answer["accepted"] == Value::Bool(false) {
            return;
        }
        let surface = text(body, "surface_id");
        if body["values"]["decision"].as_str().is_some_and(is_decision)
            && self.surface_step(surface).is_some()
            && !self.surface_decision_waits(surface)
        {
            self.surface_decisions.push(SurfaceDecision { surface: surface.to_owned() });
        }
    }

    /// Whether an action on `reference` waits to resolve its step (each
    /// change of the surface is read until it does).
    pub(super) fn surface_decision_waits(&self, reference: &str) -> bool {
        self.surface_decisions.iter().any(|d| d.surface == reference)
    }

    /// A read of `reference`: this account's actions it lists that are
    /// bound to a task step still waiting on this surface become
    /// resolutions of those steps.
    pub(super) fn surface_receipts(&mut self, reference: &str, view: &Value) {
        let me = self.directory.self_principal.clone().unwrap_or_default();
        for row in list(view, "actions") {
            let receipt = text(row, "action_receipt_id");
            if text(row, "sender_principal_id") != me
                || receipt.is_empty()
                || self.resolved_receipts.contains(receipt)
            {
                continue;
            }
            let Some(action) = row["wire_base64"]
                .as_str()
                .and_then(|w| ramflux_protocol::decode_base64url(w).ok())
                .and_then(|bytes| A2uiSurfaceAction::from_wire_bytes(&bytes).ok())
            else {
                continue;
            };
            let (Some(binding), Some(decision)) =
                (action.task_binding(), action.values()["decision"].as_str())
            else {
                continue;
            };
            if !is_decision(decision) {
                continue;
            }
            let task = super::tasks::task_key(action.conversation_id(), &binding.task_id);
            let Some(resolution) =
                self.surface_resolution(&task, &binding.step_id, reference, receipt, decision)
            else {
                continue;
            };
            self.resolved_receipts.insert(receipt.to_owned());
            self.surface_decisions.retain(|d| d.surface != reference);
            self.resolutions.push(resolution);
        }
    }

    /// The resolution of step `step` of `task` by the action receipt
    /// `receipt`, if the task is this account's, the step still waits on
    /// `reference` and no resolution of it is on its way.
    fn surface_resolution(
        &self,
        task: &TaskId,
        step: &str,
        reference: &str,
        receipt: &str,
        decision: &str,
    ) -> Option<Value> {
        let known = self.tasks.get(task)?;
        if known.shown.requester != Actor::Human
            || known.pending.as_ref().is_some_and(|(_, pending)| pending == step)
        {
            return None;
        }
        let waiting = known.shown.steps.iter().find(|s| s.id.as_str() == step)?;
        if waiting.state != StepState::NeedsMe
            || waiting.needs_me.as_ref().and_then(NeedsMe::surface).map(|s| s.as_str())
                != Some(reference)
        {
            return None;
        }
        let target = known.places.iter().find(|(s, _)| *s == waiting.id)?.1.clone();
        Some(json!({
            "task_id": task,
            "base_revision": known.shown.version,
            "step_id": waiting.id,
            "target": target,
            "decision_id": receipt,
            "decision": decision,
        }))
    }

    /// A task this account asked for now shows `step` waiting on the
    /// surface `reference`: when that surface was already read (it may have
    /// listed an action answering the step before the step was shown), it
    /// is read again.
    pub(super) fn reread_answered_surface(&mut self, reference: &str) {
        if self.surfaces.contains_key(reference)
            && !self.surface_rereads.iter().any(|r| r == reference)
        {
            self.surface_rereads.push(reference.to_owned());
        }
    }

    /// The surfaces to read again now.
    pub(super) fn take_surface_rereads(&mut self) -> Vec<crate::plan::fetch::Fetch> {
        std::mem::take(&mut self.surface_rereads)
            .into_iter()
            .map(crate::plan::fetch::Fetch::Surface)
            .collect()
    }

    /// The task resolutions to send now.
    pub fn take_task_resolutions(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.resolutions)
    }
}

fn is_decision(decision: &str) -> bool {
    matches!(decision, "resume" | "deny")
}
