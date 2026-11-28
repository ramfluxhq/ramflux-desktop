// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The join sheet (derived design D-M2-2, from the add contact dialog): a
//! field for the invite link, Paste, and Join.
//!
//! - **One call per tap.** Join is disabled while a call runs, and Return is
//!   ignored then. No outcome starts another call: a refusal, a failure, a
//!   rate limit, a timeout or a relaunch waits for the next tap (S3).
//! - **The local check** (S2) runs before the intent: a link that is not 1
//!   to 512 printable ASCII characters (after trimming ASCII whitespace)
//!   shows "link invalid" and calls nothing. The link is otherwise passed on
//!   verbatim and never parsed, previewed or opened.
//! - **Attempts.** The sheet takes the next attempt number from the store
//!   and shows an outcome only when it carries that attempt. A later outcome
//!   of an older attempt is still recorded in the store, so a joined group or
//!   a pending request is never lost; the sheet just does not show it.
//! - **Paste** reads the clipboard on the tap only. The field is never saved
//!   and is cleared when the sheet closes and after a join.

use crate::style::DesignButton as _;
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    Context, Entity, EventEmitter, FocusHandle, IntoElement, Render, Role, SharedString,
    Subscription, Window, div,
};
use ramflux_desktop_core::mls_group::{
    self as mls, JoinOutcome, JoinRefusal, PendingReason, error_key,
};
use ramflux_desktop_core::{Scope, UiEventKind};
use ramflux_desktop_navigation::scale::{space, text_secondary};

use super::{group_name, wait_text, when};
use crate::app::{StoreEvent, StoreHandle};
use crate::bubble::EventSink;
use crate::i18n::{self, tr, tr_args};

/// Reported to the workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MlsJoinEvent {
    /// Show this group.
    Open(String),
}

/// What the sheet shows under the field.
#[derive(Debug, Clone, PartialEq)]
enum Shown {
    /// Nothing yet.
    Nothing,
    /// The local check failed: no call was made.
    LocalInvalid,
    /// The call of this attempt runs.
    Joining,
    /// The outcome of this attempt.
    Outcome(JoinOutcome),
}

/// The sheet and its view-local state.
pub struct MlsJoin {
    store: Entity<StoreHandle>,
    sink: EventSink,
    open: bool,
    field: Entity<InputState>,
    /// The attempt this sheet made last (while it is open).
    attempt: Option<u64>,
    local_invalid: bool,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<MlsJoinEvent> for MlsJoin {}

impl MlsJoin {
    /// A closed sheet.
    pub fn new(
        store: Entity<StoreHandle>,
        sink: EventSink,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let field = i18n::localized_input("group.join.field", window, cx, |input| input);
        let subscriptions = vec![
            cx.subscribe_in(&store, window, |this, _, event: &StoreEvent, window, cx| {
                if matches!(event, StoreEvent::Changed(_)) {
                    this.observe(window, cx);
                }
            }),
            cx.subscribe_in(&field, window, |this, _, event, window, cx| match event {
                InputEvent::PressEnter { .. } => this.join(window, cx),
                InputEvent::Change => {
                    this.local_invalid = false;
                    cx.notify();
                }
                _ => {}
            }),
            // The window going to the background closes the sheet; the
            // outcome still lands in the store.
            cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() {
                    this.close(window, cx);
                }
            }),
        ];
        Self {
            store,
            sink: sink.scoped(Scope::Panel("mls_join".to_owned())),
            open: false,
            field,
            attempt: None,
            local_invalid: false,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// Opens the sheet with an empty field.
    pub fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = true;
        self.attempt = None;
        self.local_invalid = false;
        self.field.update(cx, |field, cx| {
            field.set_value("", window, cx);
            field.focus(window, cx);
        });
        cx.notify();
    }

    /// Closes the sheet and clears the field. A later outcome still lands in
    /// the store.
    pub fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open {
            self.open = false;
            self.attempt = None;
            self.field.update(cx, |field, cx| field.set_value("", window, cx));
            cx.notify();
        }
    }

    /// Whether the sheet is open.
    pub fn is_open(&self) -> bool {
        self.open
    }

    fn shown(&self, cx: &gpui_kit::App) -> Shown {
        if self.local_invalid {
            return Shown::LocalInvalid;
        }
        // A call of this account is in flight (made by an earlier sheet): a
        // reopened sheet shows it, and Join stays off until it ends (S3).
        let in_flight = self.store.read(cx).store().mls_join().in_flight.is_some();
        let Some(attempt) = self.attempt else {
            return if in_flight { Shown::Joining } else { Shown::Nothing };
        };
        match &self.store.read(cx).store().mls_join().last {
            Some((a, outcome)) if *a == attempt => Shown::Outcome(outcome.clone()),
            _ => Shown::Joining,
        }
    }

    fn joining(&self, cx: &gpui_kit::App) -> bool {
        self.shown(cx) == Shown::Joining
    }

    /// Paste pressed: the clipboard is read now, on the tap.
    fn paste(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.joining(cx) {
            return;
        }
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.field.update(cx, |field, cx| field.set_value(text, window, cx));
        }
    }

    /// Join pressed (or Return): checks the link locally, then asks once.
    fn join(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.open || self.joining(cx) {
            return;
        }
        // An outcome shown for an earlier attempt gives way to the next.
        let typed = self.field.read(cx).value().to_string();
        let Some(link) = mls::check_link(&typed) else {
            self.local_invalid = true;
            self.attempt = None;
            cx.notify();
            return;
        };
        self.local_invalid = false;
        let attempt = self.store.read(cx).store().mls_next_attempt();
        self.attempt = Some(attempt);
        self.sink.emit(UiEventKind::MlsJoinPressed { link, attempt }, window, cx);
        cx.notify();
    }

    /// Follows the store: a joined outcome of this sheet's attempt clears
    /// the field (S1).
    fn observe(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.open {
            return;
        }
        if matches!(self.shown(cx), Shown::Outcome(JoinOutcome::Joined { .. }))
            && !self.field.read(cx).value().is_empty()
        {
            self.field.update(cx, |field, cx| field.set_value("", window, cx));
        }
        cx.notify();
    }

    fn text(&self, outcome: &Shown, cx: &gpui_kit::App) -> Option<(SharedString, bool)> {
        let store = self.store.read(cx).store();
        match outcome {
            Shown::Nothing => None,
            Shown::LocalInvalid => Some((tr("group.join.refused.link_invalid"), true)),
            Shown::Joining => Some((tr("group.join.joining"), false)),
            Shown::Outcome(outcome) => Some(match outcome {
                JoinOutcome::Joined { group_id } => {
                    let name = store
                        .mls_group(group_id)
                        .map_or_else(|| tr("group.join.the_group"), |g| group_name(&g.name));
                    (tr_args("group.join.joined", &[("name", &name)]), false)
                }
                JoinOutcome::Pending { reason, expires_at, .. } => {
                    let time = expires_at.map(when).unwrap_or_default();
                    let key = match reason {
                        PendingReason::AwaitingMemberOnline => "group.join.pending.online",
                        _ => "group.join.pending.admin",
                    };
                    (tr_args(key, &[("time", &time)]), false)
                }
                JoinOutcome::Refused(refusal) => (refusal_text(refusal), true),
                JoinOutcome::Failed { code } => (tr(error_key(code)), true),
                JoinOutcome::Unavailable => (tr("group.error.unavailable"), true),
            }),
        }
    }
}

/// The typed text of a refusal.
fn refusal_text(refusal: &JoinRefusal) -> SharedString {
    match refusal {
        JoinRefusal::InviteExpired { expired_at } => {
            tr_args(refusal.key(), &[("time", &expired_at.map(when).unwrap_or_default())])
        }
        JoinRefusal::RateLimited { retry_after_s } => {
            tr_args(refusal.key(), &[("time", &wait_text(retry_after_s.unwrap_or(10)))])
        }
        other => tr(other.key()),
    }
}

impl Render for MlsJoin {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.open {
            return div().id("mls-join").into_any_element();
        }
        let shown = self.shown(cx);
        let joining = shown == Shown::Joining;
        let status = self.text(&shown, cx);
        let muted = cx.theme().muted_foreground;
        let danger = crate::tone::text(cx.theme().danger, cx);
        // "Open group" after a join, or when the account is a member already.
        let open_group = match &shown {
            Shown::Outcome(JoinOutcome::Joined { group_id })
            | Shown::Outcome(JoinOutcome::Refused(JoinRefusal::AlreadyMember {
                group_id: Some(group_id),
            })) => Some(group_id.clone()),
            _ => None,
        };
        // The requester of a group that admits only while somebody is online
        // learns it (no action).
        let online_note = matches!(
            &shown,
            Shown::Outcome(JoinOutcome::Pending {
                reason: PendingReason::AwaitingMemberOnline,
                ..
            })
        );
        let this = cx.weak_entity();
        let join = Button::new("mls-join-submit")
            .design_primary(cx)
            .design_label(tr("group.join.submit"))
            .loading(joining)
            .design_disabled(joining, cx)
            .on_click(cx.listener(|this, _, window, cx| this.join(window, cx)));
        let paste = Button::new("mls-join-paste")
            .design_secondary(cx)
            .design_label(tr("group.join.paste"))
            .design_disabled(joining, cx)
            .on_click(cx.listener(|this, _, window, cx| this.paste(window, cx)));
        let open_button = open_group.map(|group_id| {
            Button::new("mls-join-open")
                .design_primary(cx)
                .design_label(tr("group.join.open"))
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.emit(MlsJoinEvent::Open(group_id.clone()));
                    this.close(window, cx);
                }))
        });
        let body = v_flex()
            .id("mls-join-body")
            .test_support()
            .gap(space(3.))
            .child(
                v_flex()
                    .gap(space(1.))
                    .child(div().text_size(text_secondary()).child(tr("group.join.field")))
                    .child(
                        Input::new(&self.field)
                            .id("mls-join-field")
                            .disabled(joining)
                            .aria_label(tr("group.join.field")),
                    ),
            )
            .children(status.map(|(text, bad)| {
                div()
                    .id("mls-join-status")
                    .role(if bad { Role::Alert } else { Role::Status })
                    .aria_label(text.clone())
                    .test_support()
                    .text_size(text_secondary())
                    .text_color(if bad { danger } else { muted })
                    .child(text)
            }))
            .children(online_note.then(|| {
                div()
                    .id("mls-join-online-note")
                    .role(Role::Status)
                    .test_support()
                    .text_size(text_secondary())
                    .text_color(muted)
                    .child(tr("group.join.pending.online_note"))
            }));
        crate::modal::Modal::new(
            "mls-join",
            tr("group.join.title"),
            &self.focus,
            move |window, cx| {
                this.update(cx, |sheet, cx| sheet.close(window, cx)).ok();
            },
        )
        .width(crate::modal::width::NARROW)
        .child(body)
        .button(paste)
        .children_of_button(open_button)
        .button(join)
        .render(cx)
    }
}
