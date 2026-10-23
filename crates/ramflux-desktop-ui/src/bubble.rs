// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! UI event bubbling (rules B1 to B5).
//!
//! Each nesting level renders its children with an [`EventSink`] scoped to
//! itself. A child emits a [`UiEventKind`]; every sink on the way up prepends
//! its [`Scope`] and forwards the event, so the root receives
//! `[Workspace, Context, Moment, Surface?, Block]`. A level may consume
//! view-local behavior (expand, form edits) there (B1), or add context it
//! owns before passing the event on (B3), but never rewrite what the event
//! means. Consumed events never reach the root. The root turns business
//! events into intents (see [`crate::root`]).

use std::rc::Rc;

use gpui_kit::{App, Window};
use ramflux_desktop_core::{Scope, UiEvent, UiEventKind};

type Forward = dyn Fn(UiEvent, &mut Window, &mut App);
type Map = dyn Fn(UiEvent, &mut Window, &mut App) -> Option<UiEvent>;
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

    /// A sink that drops everything; for views rendered outside a root.
    pub fn detached() -> Self {
        Self { forward: Rc::new(|_, _, _| {}), observer: None }
    }

    /// A sink for a child level: events emitted through it gain `scope`.
    #[must_use]
    pub fn scoped(&self, scope: Scope) -> Self {
        self.scoped_map(scope, |event, _, _| Some(event))
    }

    /// A sink for a child level that may consume events at that level.
    /// `intercept` returns true when it handled the event (B1).
    #[must_use]
    pub fn scoped_with(
        &self,
        scope: Scope,
        intercept: impl Fn(&UiEvent, &mut Window, &mut App) -> bool + 'static,
    ) -> Self {
        self.scoped_map(scope, move |event, window, cx| {
            (!intercept(&event, window, cx)).then_some(event)
        })
    }

    /// A sink for a child level that may consume an event (return `None`,
    /// B1) or add context it owns before passing it on (B3).
    #[must_use]
    pub fn scoped_map(
        &self,
        scope: Scope,
        map: impl Fn(UiEvent, &mut Window, &mut App) -> Option<UiEvent> + 'static,
    ) -> Self {
        let parent = self.clone();
        let map: Rc<Map> = Rc::new(map);
        let observer = self.observer.clone();
        Self {
            forward: Rc::new(move |event: UiEvent, window: &mut Window, cx: &mut App| {
                let event = event.within(scope.clone());
                let trace = event.clone();
                match map(event, window, cx) {
                    Some(event) => (parent.forward)(event, window, cx),
                    None => {
                        if let Some(observer) = &observer {
                            observer(&trace, &scope, cx);
                        }
                    }
                }
            }),
            observer: self.observer.clone(),
        }
    }

    /// Emits an event from this level.
    pub fn emit(&self, kind: UiEventKind, window: &mut Window, cx: &mut App) {
        (self.forward)(UiEvent { scope_path: Vec::new(), kind }, window, cx);
    }
}
