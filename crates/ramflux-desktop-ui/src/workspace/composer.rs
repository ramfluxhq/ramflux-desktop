// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The composer: the primary input of the conversation field.
//!
//! This build implements message mode. Submitting emits
//! [`UiEventKind::SubmitMessage`] through the context-scoped sink; the root
//! turns it into a `SendMessage` intent. The composer never sends anything
//! itself.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, IconName, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, Context, Entity, IntoElement, Render, Subscription, Window};
use ramflux_desktop_core::UiEventKind;

use crate::bubble::EventSink;
use crate::i18n::tr;

/// The composer view.
pub struct Composer {
    input: Entity<InputState>,
    sink: Option<EventSink>,
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
        Self { input, sink: None, _subscription: subscription }
    }

    /// Targets a context through its scoped sink, or nothing.
    pub fn set_sink(&mut self, sink: Option<EventSink>, cx: &mut Context<Self>) {
        self.sink = sink;
        cx.notify();
    }

    /// The text input, for focus and tests.
    pub fn input(&self) -> &Entity<InputState> {
        &self.input
    }

    /// Emits the current text as a message and clears the input.
    pub fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sink) = self.sink.clone() else {
            return;
        };
        let text = self.input.read(cx).value().trim().to_owned();
        if text.is_empty() {
            return;
        }
        self.input.update(cx, |input, cx| input.set_value("", window, cx));
        sink.emit(UiEventKind::SubmitMessage { text }, window, cx);
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
                    .label(tr("composer.send"))
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
