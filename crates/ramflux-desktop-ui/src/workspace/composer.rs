// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The composer: the primary input of the conversation field.
//!
//! Two modes: a message ([`UiEventKind::SubmitMessage`], becoming
//! `SendMessage`) or a one-sentence request ([`UiEventKind::TaskRequested`],
//! becoming `StartTask`). Both go through the context-scoped sink; the
//! composer never sends anything itself. The mode is view-local.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Selectable as _, Sizable as _, h_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{App, Context, Entity, IntoElement, Render, Subscription, Window};
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
    _subscription: Subscription,
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
        Self { input, sink: None, mode: ComposerMode::Message, _subscription: subscription }
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
        h_flex()
            .id("composer")
            .w_full()
            .gap_2()
            .p_3()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                Button::new("composer-mode")
                    .ghost()
                    .small()
                    .selected(self.mode == ComposerMode::Task)
                    .icon(IconName::LayoutDashboard)
                    .tooltip(tr("composer.task_mode"))
                    .disabled(!enabled)
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
                    .disabled(!enabled)
                    .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
            )
    }
}

impl Composer {
    /// Focuses the text input.
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.input.update(cx, |input, cx| input.focus(window, cx));
    }
}
