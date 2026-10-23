// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The composer: the primary input of the conversation field.
//!
//! Two modes: a message ([`UiEventKind::SubmitMessage`], becoming
//! `SendMessage`) or a one-sentence request ([`UiEventKind::TaskRequested`],
//! becoming `StartTask`). Both go through the context-scoped sink; the
//! composer never sends anything itself. The mode is view-local.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Selectable as _, Sizable as _, h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, Entity, IntoElement, Render, SharedString, Subscription, Window, div,
};
use ramflux_desktop_core::UiEventKind;

use crate::bubble::EventSink;
use crate::i18n::tr;

/// What the composer submits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposerMode {
    /// A message to the context.
    Message,
    /// A one-sentence request that starts a task.
    Task,
}

/// The composer view.
pub struct Composer {
    input: Entity<InputState>,
    sink: Option<EventSink>,
    mode: ComposerMode,
    unavailable: Unavailable,
    _subscription: Subscription,
}

/// Why a mode cannot submit on the attached backend (translated reasons).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Unavailable {
    /// Why messages cannot be sent here.
    pub message: Option<SharedString>,
    /// Why tasks cannot be started here.
    pub task: Option<SharedString>,
}

impl Composer {
    /// A composer with no target context.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder(tr("composer.placeholder")));
        let subscription = cx.subscribe_in(&input, window, |this, _, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { secondary: false, shift: false }) {
                this.submit(window, cx);
            }
        });
        Self {
            input,
            sink: None,
            mode: ComposerMode::Message,
            unavailable: Unavailable::default(),
            _subscription: subscription,
        }
    }

    /// Targets a context through its scoped sink, or nothing. A new
    /// context starts in message mode.
    pub fn set_sink(
        &mut self,
        sink: Option<EventSink>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sink = sink;
        self.set_mode(ComposerMode::Message, window, cx);
        cx.notify();
    }

    /// Records which modes the backend cannot perform; their controls are
    /// shown disabled with the reason.
    pub fn set_unavailable(&mut self, unavailable: Unavailable, cx: &mut Context<Self>) {
        if self.unavailable != unavailable {
            self.unavailable = unavailable;
            cx.notify();
        }
    }

    /// Why the current mode cannot submit, if it cannot.
    pub fn unavailable_reason(&self) -> Option<&SharedString> {
        match self.mode {
            ComposerMode::Message => self.unavailable.message.as_ref(),
            ComposerMode::Task => self.unavailable.task.as_ref(),
        }
    }

    /// The text input, for focus and tests.
    pub fn input(&self) -> &Entity<InputState> {
        &self.input
    }

    /// The current mode.
    pub fn mode(&self) -> ComposerMode {
        self.mode
    }

    /// Switches between message and task mode.
    pub fn set_mode(&mut self, mode: ComposerMode, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode == mode {
            return;
        }
        self.mode = mode;
        let placeholder = tr(match mode {
            ComposerMode::Message => "composer.placeholder",
            ComposerMode::Task => "composer.task_placeholder",
        });
        self.input.update(cx, |input, cx| input.set_placeholder(placeholder, window, cx));
        cx.notify();
    }

    /// Emits the current text as a message or a task request and clears the input.
    pub fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sink) = self.sink.clone() else {
            return;
        };
        if self.unavailable_reason().is_some() {
            return;
        }
        let text = self.input.read(cx).value().trim().to_owned();
        if text.is_empty() {
            return;
        }
        self.input.update(cx, |input, cx| input.set_value("", window, cx));
        let kind = match self.mode {
            ComposerMode::Message => UiEventKind::SubmitMessage { text },
            ComposerMode::Task => UiEventKind::TaskRequested { request: text },
        };
        sink.emit(kind, window, cx);
    }
}

impl Render for Composer {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let enabled = self.sink.is_some();
        let reason = self.unavailable_reason().cloned();
        let can_send = enabled && reason.is_none();
        let task_reason = self.unavailable.task.clone();
        let row = h_flex()
            .w_full()
            .gap_2()
            .child(
                Button::new("composer-mode")
                    .ghost()
                    .small()
                    .selected(self.mode == ComposerMode::Task)
                    .icon(IconName::LayoutDashboard)
                    .accessibility_label(tr("composer.task_mode"))
                    .tooltip(task_reason.clone().unwrap_or_else(|| tr("composer.task_mode")))
                    .disabled(
                        !enabled || (task_reason.is_some() && self.mode == ComposerMode::Message),
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        let next = match this.mode {
                            ComposerMode::Message => ComposerMode::Task,
                            ComposerMode::Task => ComposerMode::Message,
                        };
                        this.set_mode(next, window, cx);
                    })),
            )
            .child(
                Input::new(&self.input)
                    .id("composer-input")
                    .aria_label(tr("composer.label"))
                    .disabled(!enabled)
                    .flex_1(),
            )
            .child(
                Button::new("composer-send")
                    .primary()
                    .icon(IconName::ArrowUp)
                    .label(tr(match self.mode {
                        ComposerMode::Message => "composer.send",
                        ComposerMode::Task => "composer.start",
                    }))
                    .disabled(!can_send)
                    .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
            );
        v_flex()
            .id("composer")
            .w_full()
            .gap_1()
            .p_3()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(row)
            .when_some(reason.filter(|_| enabled), |col, reason| {
                col.child(
                    div()
                        .id("composer-unavailable")
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(reason)
                        .test_support(),
                )
            })
    }
}

impl Composer {
    /// Focuses the text input.
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.input.update(cx, |input, cx| input.focus(window, cx));
    }
}
