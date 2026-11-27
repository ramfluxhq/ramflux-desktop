// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Policy for the card runtime (`card.session.*`): what a card intent becomes
//! on the bus, and the pool steps that come before it.
//!
//! A card's session changes identity when it resumes, so an event, slot or
//! submit on a suspended card first resumes it (the tap is consumed) and
//! sends nothing else. Event and slot bodies go as secrets, with the token
//! the held tree issued; the token is never read from the intent.

use std::collections::BTreeSet;

use serde_json::{Value, json};

use super::{IntentError, Policy};
use crate::card_event::{CardSlotOutcome, CardStatus, https_host};
use crate::ids::{MomentId, SurfaceId};
use crate::intent::{CardInput, Effect, IntentKind, SecretInput, methods};
use crate::model::{CardPhase, CardSeen, CardToken, CardTree, MomentKind};
use crate::store::{CardRecord, MAX_RESUMED, PoolPlan};

/// Effects of a card intent, or `None` for a kind of another area.
pub(super) fn handle(
    p: &Policy<'_>,
    kind: &IntentKind,
    stamp: &dyn Fn(Value) -> Value,
) -> Option<Result<Vec<Effect>, IntentError>> {
    Some(match kind {
        IntentKind::CardOpen { moment_id, surface_id, locale, dark } => {
            open(p, moment_id, surface_id).and_then(|()| {
                Ok(vec![call(
                    p,
                    methods::CARD_SESSION_OPEN,
                    json!({ "surface_id": surface_id, "locale": locale, "dark": dark }),
                    stamp,
                )?])
            })
        }
        IntentKind::CardPresent { moment_id, surface_id, locale, dark } => {
            present_target(p, moment_id, surface_id).and_then(|record| match record {
                // A suspended card is presented on its resume; the app asks
                // again then, when the runtime's locale differs.
                None => Ok(Vec::new()),
                Some(record) => Ok(vec![call(
                    p,
                    methods::CARD_SESSION_PRESENT,
                    json!({
                        "surface_id": surface_id,
                        "session": record.session,
                        "locale": locale,
                        "dark": dark,
                    }),
                    stamp,
                )?]),
            })
        }
        IntentKind::CardInteract { moment_id, surface_id, key, event, seen } => {
            p.human_only("act on a card").and_then(|()| {
                interact(p, moment_id, surface_id, Taken { target: key, seen }, event, stamp)
            })
        }
        IntentKind::CardSlotAnswer { moment_id, surface_id, slot_id, outcome, seen } => {
            p.human_only("answer a card").and_then(|()| {
                slot_answer(
                    p,
                    moment_id,
                    surface_id,
                    Taken { target: slot_id, seen },
                    *outcome,
                    stamp,
                )
            })
        }
        IntentKind::CardSubmit { moment_id, surface_id } => {
            p.human_only("submit a card").and_then(|()| submit(p, moment_id, surface_id, stamp))
        }
        IntentKind::CardRefresh { moment_id, surface_id } => {
            live_or_touch(p, moment_id, surface_id, stamp, |record| {
                call(
                    p,
                    methods::CARD_SESSION_UPDATE,
                    json!({ "surface_id": surface_id, "session": record.session }),
                    stamp,
                )
                .map(|effect| vec![effect])
            })
        }
        IntentKind::CardSuspend { moment_id, surface_id } => {
            present_target(p, moment_id, surface_id).and_then(|record| match record {
                Some(record) => Ok(vec![call(
                    p,
                    methods::CARD_SESSION_SUSPEND,
                    json!({ "surface_id": surface_id, "session": record.session }),
                    stamp,
                )?]),
                None => Ok(Vec::new()),
            })
        }
        IntentKind::CardDispose { moment_id, surface_id } => {
            // A suspended card is released too: disposing is how the pool lets go.
            check_moment(p, moment_id, surface_id).and_then(|()| {
                let record = p
                    .store
                    .card(surface_id)
                    .ok_or_else(|| IntentError::NotActionable(moment_id.clone()))?;
                Ok(vec![call(
                    p,
                    methods::CARD_SESSION_DISPOSE,
                    json!({ "surface_id": surface_id, "session": record.session }),
                    stamp,
                )?])
            })
        }
        IntentKind::CardSuspendAll => {
            let mut pool = p.store.card_pool().clone();
            let plan = pool.suspend_all();
            steps(p, plan, stamp)
        }
        IntentKind::CardResumeAll => {
            let mut pool = p.store.card_pool().clone();
            let mut plan = PoolPlan::default();
            // Least recent first, so the most recent card ends up resumed.
            for card in p.store.card_pool().recent(MAX_RESUMED).iter().rev() {
                let next = pool.touch(card);
                plan.suspend.extend(next.suspend);
                plan.resume = next.resume.or(plan.resume);
                plan.dispose.extend(next.dispose);
            }
            steps(p, plan, stamp)
        }
        IntentKind::CardRetainVisible { surface_ids } => {
            let mut pool = p.store.card_pool().clone();
            let visible: BTreeSet<String> =
                surface_ids.iter().map(|s| s.as_str().to_owned()).collect();
            let plan = pool.retain_visible(&visible);
            steps(p, plan, stamp)
        }
        _ => return None,
    })
}

/// The moment holds `surface`, and the moment is an agent surface.
fn check_moment(
    p: &Policy<'_>,
    moment_id: &MomentId,
    surface_id: &SurfaceId,
) -> Result<(), IntentError> {
    let moment = p.moment(moment_id)?;
    if !matches!(moment.kind, MomentKind::AgentSurface(_)) {
        return Err(IntentError::WrongMomentKind {
            moment: moment_id.clone(),
            expected: "AgentSurface",
        });
    }
    if p.store.surface_moment(surface_id) != Some(moment_id) {
        return Err(IntentError::NotActionable(moment_id.clone()));
    }
    Ok(())
}

/// The held card of `surface` on `moment_id`, or `None` when it is suspended
/// in the pool (`Some` only for a resumed card).
fn present_target<'a>(
    p: &'a Policy<'_>,
    moment_id: &MomentId,
    surface_id: &SurfaceId,
) -> Result<Option<&'a CardRecord>, IntentError> {
    check_moment(p, moment_id, surface_id)?;
    let record =
        p.store.card(surface_id).ok_or_else(|| IntentError::NotActionable(moment_id.clone()))?;
    Ok(record.resumed(p.store.card_pool(), surface_id).then_some(record))
}

fn open(p: &Policy<'_>, moment_id: &MomentId, surface_id: &SurfaceId) -> Result<(), IntentError> {
    check_moment(p, moment_id, surface_id)?;
    if p.store.card(surface_id).is_some() {
        return Err(IntentError::NotActionable(moment_id.clone()));
    }
    Ok(())
}

/// Runs `then` on a resumed card; a suspended one is resumed instead.
fn live_or_touch(
    p: &Policy<'_>,
    moment_id: &MomentId,
    surface_id: &SurfaceId,
    stamp: &dyn Fn(Value) -> Value,
    then: impl FnOnce(&CardRecord) -> Result<Vec<Effect>, IntentError>,
) -> Result<Vec<Effect>, IntentError> {
    check_moment(p, moment_id, surface_id)?;
    let record =
        p.store.card(surface_id).ok_or_else(|| IntentError::NotActionable(moment_id.clone()))?;
    if record.resumed(p.store.card_pool(), surface_id) {
        then(record)
    } else {
        touch(p, surface_id, stamp)
    }
}

/// The pool steps that make `surface` resumed: suspends for the least
/// recently used cards over the limit, then the resume.
fn touch(
    p: &Policy<'_>,
    surface_id: &SurfaceId,
    stamp: &dyn Fn(Value) -> Value,
) -> Result<Vec<Effect>, IntentError> {
    let mut pool = p.store.card_pool().clone();
    let plan = pool.touch(surface_id.as_str());
    steps(p, plan, stamp)
}

/// What the user saw when a host event or slot answer was taken: the node
/// it addressed (or the slot id) and the tree state.
#[derive(Clone, Copy)]
struct Taken<'a> {
    target: &'a str,
    seen: &'a CardSeen,
}

/// The token a host event on `key` is sent with: the held tree's, once
/// `seen` still names the held version. A text draft (`rebase`) may also
/// name an older version of this desktop's own chain, where its node keeps
/// its kind and input target; the whole value then replaces the draft
/// (last write wins). Anything else is a change the user has not seen.
fn held_token(
    record: &CardRecord,
    tree: &CardTree,
    key: &str,
    seen: &CardSeen,
    rebase: bool,
    moment_id: &MomentId,
) -> Result<CardToken, IntentError> {
    let same = seen.stamp == tree.stamp();
    if !same && !(rebase && record.rebasable(key, seen)) {
        return Err(IntentError::CardChanged(moment_id.clone()));
    }
    tree.token_for(key).ok_or_else(|| IntentError::NotActionable(moment_id.clone()))
}

/// The held tree's stamp, when `seen` still names it (a strict check for
/// taps, tabs, modals and slot answers).
fn held_stamp(tree: &CardTree, seen: &CardSeen, moment_id: &MomentId) -> Result<(), IntentError> {
    if seen.stamp == tree.stamp() {
        Ok(())
    } else {
        Err(IntentError::CardChanged(moment_id.clone()))
    }
}

/// A live card's host event: its token from the held tree, the event as a
/// secret.
fn interact(
    p: &Policy<'_>,
    moment_id: &MomentId,
    surface_id: &SurfaceId,
    taken: Taken<'_>,
    event: &CardInput,
    stamp: &dyn Fn(Value) -> Value,
) -> Result<Vec<Effect>, IntentError> {
    live_or_touch(p, moment_id, surface_id, stamp, |record| {
        let tree = interactive_tree(record, moment_id)?;
        let rebase = matches!(event, CardInput::Input { .. });
        let key = taken.target;
        let token = held_token(record, tree, key, taken.seen, rebase, moment_id)?;
        let event = serde_json::to_value(event)
            .map_err(|_| IntentError::NotActionable(moment_id.clone()))?;
        let secret = json!({ "token": token, "event": event }).to_string();
        Ok(vec![secret_call(
            p,
            methods::CARD_SESSION_EVENT,
            json!({ "surface_id": surface_id, "session": record.session }),
            SecretInput::new(secret),
            stamp,
        )?])
    })
}

/// A host answer to a slot: the link the runtime asked for, or an approval
/// slot of the held tree. The request id and the token are the secret.
fn slot_answer(
    p: &Policy<'_>,
    moment_id: &MomentId,
    surface_id: &SurfaceId,
    taken: Taken<'_>,
    outcome: CardSlotOutcome,
    stamp: &dyn Fn(Value) -> Value,
) -> Result<Vec<Effect>, IntentError> {
    live_or_touch(p, moment_id, surface_id, stamp, |record| {
        let tree = interactive_tree(record, moment_id)?;
        let slot_id = taken.target;
        held_stamp(tree, taken.seen, moment_id)?;
        // A link is opened only over https (the sheet refuses other links).
        if let Some(link) = record.link.as_ref().filter(|l| l.slot_id == slot_id)
            && outcome == CardSlotOutcome::Confirm
            && https_host(&link.url).is_none()
        {
            return Err(IntentError::NotActionable(moment_id.clone()));
        }
        // The URL the sheet shows, so the answer can name the link it opened.
        let url = record.link.as_ref().filter(|l| l.slot_id == slot_id).map(|l| l.url.clone());
        let (token, request_id) = match &record.link {
            Some(link) if link.slot_id == slot_id => (link.token.clone(), link.request_id.clone()),
            _ => {
                let tree = record
                    .tree
                    .as_ref()
                    .ok_or_else(|| IntentError::NotActionable(moment_id.clone()))?;
                let slot = tree
                    .host_slots()
                    .iter()
                    .find(|s| s.id == slot_id)
                    .ok_or_else(|| IntentError::NotActionable(moment_id.clone()))?;
                let request_id = slot
                    .request
                    .get("request_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let token = serde_json::to_value(&slot.token)
                    .map_err(|_| IntentError::NotActionable(moment_id.clone()))?;
                (token, request_id)
            }
        };
        let secret =
            json!({ "token": token, "slot_id": slot_id, "request_id": request_id }).to_string();
        Ok(vec![secret_call(
            p,
            methods::CARD_SESSION_SLOT,
            json!({
                "surface_id": surface_id,
                "session": record.session,
                "outcome": outcome_wire(outcome),
                "url": url,
            }),
            SecretInput::new(secret),
            stamp,
        )?])
    })
}

/// The submit a button press asked for; only after the runtime consumed the
/// tap (`PendingSubmit`).
fn submit(
    p: &Policy<'_>,
    moment_id: &MomentId,
    surface_id: &SurfaceId,
    stamp: &dyn Fn(Value) -> Value,
) -> Result<Vec<Effect>, IntentError> {
    live_or_touch(p, moment_id, surface_id, stamp, |record| {
        if record.status != CardStatus::PendingSubmit {
            return Err(IntentError::NotActionable(moment_id.clone()));
        }
        Ok(vec![call(
            p,
            methods::CARD_SESSION_SUBMIT,
            json!({ "surface_id": surface_id, "session": record.session }),
            stamp,
        )?])
    })
}

/// The tree of a card the host may interact with now: live and interactive.
fn interactive_tree<'a>(
    record: &'a CardRecord,
    moment_id: &MomentId,
) -> Result<&'a crate::model::CardTree, IntentError> {
    let not = || IntentError::NotActionable(moment_id.clone());
    if record.status != CardStatus::Live {
        return Err(not());
    }
    let tree = record.tree.as_ref().ok_or_else(not)?;
    if tree.phase() != CardPhase::Ready {
        return Err(not());
    }
    Ok(tree)
}

fn outcome_wire(outcome: CardSlotOutcome) -> &'static str {
    match outcome {
        CardSlotOutcome::Confirm => "confirm",
        CardSlotOutcome::Deny => "deny",
        CardSlotOutcome::Cancel => "cancel",
        CardSlotOutcome::Unavailable => "unavailable",
    }
}

/// The effects of a pool plan: suspends, then the resume, then disposes.
fn steps(
    p: &Policy<'_>,
    plan: PoolPlan,
    stamp: &dyn Fn(Value) -> Value,
) -> Result<Vec<Effect>, IntentError> {
    let mut effects = Vec::new();
    for (card, session) in plan.suspend {
        effects.push(call(
            p,
            methods::CARD_SESSION_SUSPEND,
            json!({ "surface_id": card, "session": session }),
            stamp,
        )?);
    }
    if let Some((card, session)) = plan.resume {
        effects.push(call(
            p,
            methods::CARD_SESSION_RESUME,
            json!({ "surface_id": card, "session": session }),
            stamp,
        )?);
    }
    for (card, session) in plan.dispose {
        effects.push(call(
            p,
            methods::CARD_SESSION_DISPOSE,
            json!({ "surface_id": card, "session": session }),
            stamp,
        )?);
    }
    Ok(effects)
}

/// A card call, refused when the backend does not perform it.
fn call(
    p: &Policy<'_>,
    method: &'static str,
    body: Value,
    stamp: &dyn Fn(Value) -> Value,
) -> Result<Effect, IntentError> {
    if let Some(reason) = p.store.unsupported_reason(method) {
        return Err(IntentError::UnsupportedByBackend { method, reason: reason.to_owned() });
    }
    Ok(Effect::Call { method: method.to_owned(), body: stamp(body) })
}

/// A card call that carries a secret (event or slot).
fn secret_call(
    p: &Policy<'_>,
    method: &'static str,
    body: Value,
    secret: SecretInput,
    stamp: &dyn Fn(Value) -> Value,
) -> Result<Effect, IntentError> {
    if let Some(reason) = p.store.unsupported_reason(method) {
        return Err(IntentError::UnsupportedByBackend { method, reason: reason.to_owned() });
    }
    Ok(Effect::SecretCall { method: method.to_owned(), body: stamp(body), secrets: vec![secret] })
}
