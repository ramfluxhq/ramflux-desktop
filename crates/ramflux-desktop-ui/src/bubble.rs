// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! UI event bubbling (rules B1 to B5).
//!
//! Each nesting level renders its children with an [`EventSink`] scoped to
//! itself. A child emits a [`UiEventKind`]; every sink on the way up prepends
//! its [`Scope`] and forwards the event, so the root receives
//! `[Workspace, Context, Moment, Surface?, Block]`. A level may intercept
//! view-local behavior (expand, collapse) and consume it there; consumed events
//! never reach the root. The root turns business events into intents with
//! [`intent_for`].

use std::rc::Rc;

use gpui_kit::{App, Window};
use ramflux_desktop_core::{IntentKind, MomentKind, Scope, Store, UiEvent, UiEventKind};

type Forward = dyn Fn(UiEvent, &mut Window, &mut App);
type Intercept = dyn Fn(&UiEvent, &mut Window, &mut App) -> bool;
type Observer = dyn Fn(&UiEvent, &Scope, &mut App);

/// A cheap clonable callback chain from one UI level to the root.
#[derive(Clone)]
pub struct EventSink {
    forward: Rc<Forward>,
    observer: Option<Rc<Observer>>,
}

impl EventSink {
    /// The root sink. `handler` receives every event that was not consumed
    /// below, with [`Scope::Workspace`] prepended. `observer` is told about
    /// events consumed below the root (for the event inspector, B5).
    pub fn root(
        handler: impl Fn(UiEvent, &mut Window, &mut App) + 'static,
        observer: impl Fn(&UiEvent, &Scope, &mut App) + 'static,
    ) -> Self {
        Self {
            forward: Rc::new(move |event: UiEvent, window: &mut Window, cx: &mut App| {
                handler(event.within(Scope::Workspace), window, cx);
            }),
            observer: Some(Rc::new(observer)),
        }
    }

    /// A sink that drops everything; for views rendered outside a workspace.
    pub fn detached() -> Self {
        Self { forward: Rc::new(|_, _, _| {}), observer: None }
    }

    /// A sink for a child level: events emitted through it gain `scope`.
    #[must_use]
    pub fn scoped(&self, scope: Scope) -> Self {
        self.scoped_with(scope, |_, _, _| false)
    }

    /// A sink for a child level that may consume events at that level.
    /// `intercept` returns true when it handled the event (B1).
    #[must_use]
    pub fn scoped_with(
        &self,
        scope: Scope,
        intercept: impl Fn(&UiEvent, &mut Window, &mut App) -> bool + 'static,
    ) -> Self {
        let parent = self.clone();
        let intercept: Rc<Intercept> = Rc::new(intercept);
        let observer = self.observer.clone();
        Self {
            forward: Rc::new(move |event: UiEvent, window: &mut Window, cx: &mut App| {
                let event = event.within(scope.clone());
                if intercept(&event, window, cx) {
                    if let Some(observer) = &observer {
                        observer(&event, &scope, cx);
                    }
                    return;
                }
                (parent.forward)(event, window, cx);
            }),
            observer: self.observer.clone(),
        }
    }

    /// Emits an event from this level.
    pub fn emit(&self, kind: UiEventKind, window: &mut Window, cx: &mut App) {
        (self.forward)(UiEvent { scope_path: Vec::new(), kind }, window, cx);
    }
}

/// Maps a root-level UI event to the intent it requests.
///
/// Returns `None` for view-local events and for events whose scope does not
/// identify a valid target. A UI event can never carry human confirmation:
/// approvals and surface actions always map with `confirmed: false`, so the
/// only path to a confirmed intent is the hold-to-confirm strip.
pub fn intent_for(event: &UiEvent, store: &Store) -> Option<IntentKind> {
    let context_id = || event.context_id().cloned();
    let moment = || event.moment_id().and_then(|id| store.moment(id));
    match &event.kind {
        UiEventKind::SubmitMessage { text } => {
            Some(IntentKind::SendMessage { context_id: context_id()?, text: text.clone() })
        }
        UiEventKind::AttachFile { path } => {
            Some(IntentKind::AttachObject { context_id: context_id()?, path: path.clone() })
        }
        UiEventKind::TaskRequested { request } => {
            Some(IntentKind::StartTask { context_id: context_id()?, request: request.clone() })
        }
        UiEventKind::PinToggled { pinned } => {
            let context_id = context_id()?;
            Some(if *pinned {
                IntentKind::PinContext { context_id }
            } else {
                IntentKind::UnpinContext { context_id }
            })
        }
        UiEventKind::ActionInvoked { action_id, values } => {
            let MomentKind::AgentSurface(surface) = &moment()?.kind else { return None };
            Some(IntentKind::SurfaceAction {
                moment_id: event.moment_id()?.clone(),
                surface_id: event.surface_id().unwrap_or(&surface.surface_id).clone(),
                action_id: action_id.clone(),
                values: values.clone(),
                confirmed: false,
            })
        }
        UiEventKind::ApprovePressed { .. } | UiEventKind::DenyPressed => {
            let MomentKind::Intervention(payload) = &moment()?.kind else { return None };
            let grant_id = payload.grant_id.clone();
            Some(match event.kind {
                UiEventKind::DenyPressed => IntentKind::Deny { grant_id },
                _ => IntentKind::Approve { grant_id, confirmed: false },
            })
        }
        UiEventKind::RevealPressed => Some(IntentKind::Reveal { moment_id: moment()?.id.clone() }),
        UiEventKind::ConflictResolved { resolution } => Some(IntentKind::ResolveConflict {
            moment_id: moment()?.id.clone(),
            resolution: *resolution,
        }),
        UiEventKind::CancelPressed => {
            let MomentKind::TaskProgress(payload) = &moment()?.kind else { return None };
            Some(IntentKind::CancelTask { task_id: payload.task_id.clone() })
        }
        UiEventKind::ContactAnswered { accept } => {
            let moment_id = moment()?.id.clone();
            Some(if *accept {
                IntentKind::AcceptContact { moment_id }
            } else {
                IntentKind::RejectContact { moment_id }
            })
        }
        UiEventKind::ExpandToggled { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use ramflux_desktop_core::{BlockId, ContextId, GrantId, MomentId};
    use ramflux_desktop_test::ScenarioPlayer;
    use ramflux_desktop_test::load_fixture;

    use super::*;

    fn s03_store() -> ScenarioPlayer {
        let mut player = ScenarioPlayer::new(&load_fixture("s03_intervention").expect("s03"));
        player.advance(1_000);
        player
    }

    fn at_moment(kind: UiEventKind) -> UiEvent {
        UiEvent::new(Scope::Block(BlockId::new("decision")), kind)
            .within(Scope::Moment(MomentId::new("m-approve")))
            .within(Scope::Context(ContextId::new("c-ops")))
            .within(Scope::Workspace)
    }

    #[test]
    fn a_claimed_confirmation_from_the_ui_is_ignored() {
        let player = s03_store();
        let event = at_moment(UiEventKind::ApprovePressed { confirmed: true });
        let intent = intent_for(&event, player.store());
        assert!(matches!(
            intent,
            Some(IntentKind::Approve { ref grant_id, confirmed: false }) if *grant_id == GrantId::new("g-rotate")
        ));
    }

    #[test]
    fn view_local_and_untargeted_events_map_to_nothing() {
        let player = s03_store();
        assert!(
            intent_for(&at_moment(UiEventKind::ExpandToggled { expanded: true }), player.store())
                .is_none()
        );
        let orphan =
            UiEvent::new(Scope::Workspace, UiEventKind::SubmitMessage { text: "x".into() });
        assert!(intent_for(&orphan, player.store()).is_none());
        let wrong_kind = at_moment(UiEventKind::CancelPressed);
        assert!(intent_for(&wrong_kind, player.store()).is_none());
    }
}
