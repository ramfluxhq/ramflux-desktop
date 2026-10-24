// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! A choice or an input a task step asks for, answered in the assignee's
//! surface, then resolved on the task.
//!
//! The step names the assignee's surface; the user acts on that surface
//! as on any other (`surface.action`), with values that say
//! `decision: "resume"` or `"deny"`. That action is remembered. Once a
//! read of the surface lists it (`surface.get` `actions[]`: this
//! account's action with that version, action id and decision) with
//! `receipt_verified: true` (the author's creator receipt is stored), the
//! step is resolved with `task.resolve`, `decision_id` being the row's
//! `action_receipt_id` (the delivery receipt, not the action name). Until
//! the receipt is there the surface is read on each `surface.changed`.
//! Each receipt resolves at most one step, once.

use ramflux_desktop_core::{Actor, NeedsMe, StepId, StepState, TaskId, methods};
use ramflux_protocol::A2uiSurfaceAction;
use serde_json::{Value, json};

use super::{Translator, list, text};

/// A surface action that answers a task step, waiting for its receipt.
#[derive(Debug, Clone)]
pub(crate) struct SurfaceDecision {
    /// The surface (`surface_ref`).
    surface: String,
    /// The version acted on.
    version: u64,
    /// The action.
    action_id: String,
    /// `resume` or `deny`.
    decision: String,
    /// The task and its step.
    task: TaskId,
    step: StepId,
}

impl Translator {
    /// After a surface action the daemon accepted: if its values decide a
    /// step of a task this account asked for that waits on this surface,
    /// the action is remembered until its receipt arrives.
    pub(super) fn note_surface_decision(&mut self, method: &str, body: &Value, answer: &Value) {
        if method != methods::SURFACE_ACTION || answer["accepted"] == Value::Bool(false) {
            return;
        }
        let surface = text(body, "surface_id");
        let Some(decision) = body["values"]["decision"].as_str().filter(|d| is_decision(d)) else {
            return;
        };
        let Some(version) = body["version"].as_u64() else { return };
        let step = self.tasks.iter().find_map(|(id, known)| {
            (known.shown.requester == Actor::Human)
                .then(|| {
                    known.shown.steps.iter().find(|s| {
                        s.state == StepState::NeedsMe
                            && s.needs_me.as_ref().and_then(NeedsMe::surface).map(|s| s.as_str())
                                == Some(surface)
                    })
                })
                .flatten()
                .map(|s| (id.clone(), s.id.clone()))
        });
        let Some((task, step)) = step else { return };
        self.surface_decisions.push(SurfaceDecision {
            surface: surface.to_owned(),
            version,
            action_id: text(body, "action_id").to_owned(),
            decision: decision.to_owned(),
            task,
            step,
        });
    }

    /// Whether an action on `reference` waits for its receipt (each change
    /// of the surface is read until it arrives).
    pub(super) fn surface_decision_waits(&self, reference: &str) -> bool {
        self.surface_decisions.iter().any(|d| d.surface == reference)
    }

    /// A read of `reference`: the remembered actions it lists with a
    /// verified receipt become resolutions of their steps.
    pub(super) fn surface_receipts(&mut self, reference: &str, view: &Value) {
        if !self.surface_decision_waits(reference) {
            return;
        }
        let me = self.directory.self_principal.clone().unwrap_or_default();
        for row in list(view, "actions") {
            let receipt = text(row, "action_receipt_id");
            if row["receipt_verified"] != Value::Bool(true)
                || text(row, "sender_principal_id") != me
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
            let found = self.surface_decisions.iter().position(|d| {
                d.surface == reference
                    && d.version == action.version()
                    && d.action_id == action.action_id()
                    && action.values()["decision"] == d.decision.as_str()
            });
            let Some(found) = found else { continue };
            let decision = self.surface_decisions.remove(found);
            self.resolved_receipts.insert(receipt.to_owned());
            if let Some(resolution) = self.surface_resolution(&decision, receipt) {
                self.resolutions.push(resolution);
            }
        }
    }

    /// The resolution of the step `decision` answered, if it still waits.
    fn surface_resolution(&self, decision: &SurfaceDecision, receipt: &str) -> Option<Value> {
        let known = self.tasks.get(&decision.task)?;
        let step = known.shown.steps.iter().find(|s| s.id == decision.step)?;
        if step.state != StepState::NeedsMe {
            return None;
        }
        let target = known.places.iter().find(|(s, _)| *s == step.id)?.1.clone();
        Some(json!({
            "task_id": decision.task,
            "base_revision": known.shown.version,
            "step_id": step.id,
            "target": target,
            "decision_id": receipt,
            "decision": decision.decision,
        }))
    }

    /// The task resolutions to send now.
    pub fn take_task_resolutions(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.resolutions)
    }
}

fn is_decision(decision: &str) -> bool {
    matches!(decision, "resume" | "deny")
}
