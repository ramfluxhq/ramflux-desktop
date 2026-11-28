// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The identity change sheet (derived design D-M1-1, from B10 and F10): the
//! one place a changed contact identity is accepted.
//!
//! It opens on "Review" (the header banner, the composer prompt, the contact
//! profile, the contact rows, a gated call) and shows the contact's name and
//! @username, the AI marker when the SDK flags the contact as an AI, what a
//! change can mean, and two buttons: "Accept new identity" and "Not now".
//! There is no safety number, code or QR to compare.
//!
//! Accepting needs one explicit click, on the anchor the sheet displayed:
//! the sheet keeps the anchor of the change it last showed, sends exactly
//! that anchor, and when the store names another one (the identity changed
//! again) it shows the new change and asks for a new click. While the
//! acceptance runs both buttons are disabled and the primary one is busy;
//! a refusal is shown inline (`identity.accept.*`). Nothing accepts by
//! itself: no timer, no accept on open, no accept from a notification, no
//! batch accept, and a message refused for the change is never sent again by
//! the acceptance. Escape, the scrim and the window losing focus close the
//! sheet without action. A contact whose change is not known yet (the
//! status says `changed` before the event arrived) has the primary button
//! disabled and "Waiting for the new identity…".

use crate::style::DesignButton as _;
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::StyledExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, Context, Entity, FocusHandle, IntoElement, Render, Subscription, Window, div,
};
use ramflux_desktop_core::{AcceptFailure, ParticipantId, Scope, Store, UiEventKind};
use ramflux_desktop_navigation::direction::h_flex;
use ramflux_desktop_navigation::scale::{space, text_caption, text_secondary};

use crate::app::{StoreEvent, StoreHandle};
use crate::bubble::EventSink;
use crate::i18n::{self, tr, tr_args};

/// The scope the sheet's events carry.
const SCOPE: &str = "identity-change";

/// What the sheet says under its text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Note {
    /// The identity changed again since the sheet was shown.
    Again,
    /// The change was handled already.
    Already,
    /// The acceptance failed.
    Failed,
}

/// The sheet and its view-local state.
pub struct IdentitySheet {
    store: Entity<StoreHandle>,
    sink: EventSink,
    /// The contact being reviewed; `None` while closed.
    participant: Option<ParticipantId>,
    /// The anchor of the change the sheet last displayed: what Accept sends.
    shown: Option<String>,
    /// The acceptance in flight: the failure number it was sent after.
    accepting: Option<u64>,
    note: Option<Note>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl IdentitySheet {
    /// A closed sheet reading `store`, emitting through `sink` (the root's).
    pub fn new(
        store: Entity<StoreHandle>,
        sink: EventSink,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = vec![
            cx.subscribe(&store, |this, _, event: &StoreEvent, cx| {
                if matches!(event, StoreEvent::Changed(_)) {
                    this.observe(cx);
                }
            }),
            // The window going to the background closes the sheet, without
            // accepting anything.
            cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() {
                    this.close(cx);
                }
            }),
        ];
        Self {
            store,
            sink: sink.scoped(Scope::Panel(SCOPE.to_owned())),
            participant: None,
            shown: None,
            accepting: None,
            note: None,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// Whether the sheet is open.
    pub fn is_open(&self) -> bool {
        self.participant.is_some()
    }

    /// The contact being reviewed, for tests.
    pub fn participant(&self) -> Option<&ParticipantId> {
        self.participant.as_ref()
    }

    /// Opens the sheet for `participant` (Review pressed). It shows the
    /// change as the store holds it now and asks for a click on that.
    pub fn open(
        &mut self,
        participant: ParticipantId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let store = self.store.read(cx).store();
        if !store.participant(&participant).is_some_and(|p| p.identity.is_changed()) {
            return;
        }
        self.shown = store
            .participant(&participant)
            .and_then(|p| p.pending_change.as_ref())
            .map(|c| c.new_anchor.clone());
        self.participant = Some(participant);
        self.accepting = None;
        self.note = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Closes the sheet without action.
    pub fn close(&mut self, cx: &mut Context<Self>) {
        if self.participant.take().is_some() {
            self.shown = None;
            self.accepting = None;
            self.note = None;
            cx.notify();
        }
    }

    /// The anchor of the change the store holds for the reviewed contact.
    fn current(store: &Store, participant: &ParticipantId) -> Option<String> {
        store
            .participant(participant)
            .and_then(|p| p.pending_change.as_ref())
            .map(|c| c.new_anchor.clone())
    }

    /// Follows the store: the change was accepted or handled elsewhere (the
    /// sheet closes), the identity changed again (the new change is shown and
    /// needs a new click), or the acceptance was refused.
    fn observe(&mut self, cx: &mut Context<Self>) {
        let Some(participant) = self.participant.clone() else { return };
        let store = self.store.read(cx).store();
        let changed = store.participant(&participant).is_some_and(|p| p.identity.is_changed());
        let failure = store.identity_failure(&participant);
        let current = Self::current(store, &participant);
        let seq = store.identity_failure_seq();
        let mut note = self.note;
        let mut accepting = self.accepting;
        if let (Some(base), Some((reason, number))) = (accepting, failure)
            && number > base
        {
            accepting = None;
            note = Some(match reason {
                AcceptFailure::AlreadyHandled => Note::Already,
                AcceptFailure::ChangedAgain => Note::Again,
                AcceptFailure::Failed => Note::Failed,
            });
        }
        let _ = seq;
        if current != self.shown {
            // Another change than the one shown: show it, and say so when
            // the user had been shown one before.
            if self.shown.is_some() && current.is_some() {
                note = Some(Note::Again);
            }
            self.shown = current;
            accepting = None;
        }
        if !changed && note != Some(Note::Already) {
            // Accepted (or resolved by a read): nothing left to review.
            self.close(cx);
            return;
        }
        self.note = note;
        self.accepting = accepting;
        cx.notify();
    }

    /// Accept pressed: sends the anchor the sheet displayed, once.
    fn accept(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(participant), Some(shown)) = (self.participant.clone(), self.shown.clone())
        else {
            return;
        };
        if self.accepting.is_some() {
            return;
        }
        let store = self.store.read(cx).store();
        // The store names another change than the one displayed: show it and
        // wait for a new click; never accept a change the user did not see.
        if Self::current(store, &participant).as_deref() != Some(shown.as_str()) {
            self.observe(cx);
            return;
        }
        self.accepting = Some(store.identity_failure_seq());
        self.note = None;
        self.sink.emit(
            UiEventKind::AcceptIdentityPressed { participant_id: participant, new_anchor: shown },
            window,
            cx,
        );
        cx.notify();
    }

    fn note_text(&self) -> Option<gpui_kit::SharedString> {
        self.note.map(|note| {
            tr(match note {
                Note::Again => "identity.accept.again",
                Note::Already => "identity.accept.already",
                Note::Failed => "identity.accept.failed",
            })
        })
    }
}

impl Render for IdentitySheet {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(participant) = self.participant.clone() else {
            return div().absolute().into_any_element();
        };
        let store = self.store.read(cx).store();
        let Some(person) = store.participant(&participant).cloned() else {
            return div().absolute().into_any_element();
        };
        let username = person
            .pending_change
            .as_ref()
            .and_then(|c| c.username.as_deref())
            .map(|u| i18n::bidi::isolate(&format!("@{u}")));
        let ai = person.ai_flag || person.kind == ramflux_desktop_core::ParticipantKind::Agent;
        let waiting = self.shown.is_none();
        let accepting = self.accepting.is_some();
        let handled = self.note == Some(Note::Already);
        let note = self.note_text();
        let muted = cx.theme().muted_foreground;
        let danger = cx.theme().danger;
        let name = person.display.clone();
        let body = v_flex()
            .id("identity-sheet-body")
            .gap(space(3.))
            .child(
                h_flex()
                    .gap(space(2.))
                    .flex_wrap()
                    .child(
                        div()
                            .id("identity-sheet-name")
                            .test_support()
                            .text_size(text_secondary())
                            .font_semibold()
                            .child(i18n::bidi::isolate(&name)),
                    )
                    .children(username.map(|username| {
                        div()
                            .id("identity-sheet-username")
                            .test_support()
                            .text_size(text_caption())
                            .text_color(muted)
                            .child(username)
                    }))
                    .children(ai.then(|| crate::identity::ai_mark("identity-sheet-ai", cx))),
            )
            .child(
                div()
                    .id("identity-sheet-text")
                    .test_support()
                    .text_size(text_secondary())
                    .whitespace_normal()
                    .child(tr_args("identity.sheet.body", &[("name", &name)])),
            )
            .children((waiting && !handled).then(|| {
                div()
                    .id("identity-sheet-waiting")
                    .role(gpui_kit::Role::Status)
                    .test_support()
                    .text_size(text_caption())
                    .text_color(muted)
                    .child(tr("identity.sheet.waiting"))
            }))
            .children(note.map(|text| {
                div()
                    .id("identity-sheet-note")
                    .role(gpui_kit::Role::Alert)
                    .aria_label(text.clone())
                    .test_support()
                    .text_size(text_caption())
                    .text_color(danger)
                    .child(text)
            }));
        let this = cx.weak_entity();
        let close = move |_: &mut Window, cx: &mut gpui_kit::App| {
            this.update(cx, |sheet, cx| sheet.close(cx)).ok();
        };
        let later = {
            let this = cx.weak_entity();
            Button::new("identity-sheet-later")
                .design_secondary(cx)
                .design_label(tr("identity.sheet.later"))
                .design_disabled(accepting, cx)
                .on_click(move |_, _, cx| {
                    this.update(cx, |sheet, cx| sheet.close(cx)).ok();
                })
        };
        let accept = Button::new("identity-sheet-accept")
            .design_primary(cx)
            .design_label(tr("identity.sheet.accept"))
            .loading(accepting)
            .design_disabled(waiting || accepting || handled, cx)
            .on_click(cx.listener(|this, _, window, cx| this.accept(window, cx)));
        crate::modal::Modal::new(
            "identity-sheet",
            tr_args("identity.sheet.title", &[("name", &name)]),
            &self.focus,
            close,
        )
        .width(crate::modal::width::NARROW)
        .child(body)
        .button(later)
        .button(accept)
        .render(cx)
    }
}

/// The sheet as an element, for the workspace to draw over the window (it
/// draws nothing while closed).
pub fn element(sheet: &Entity<IdentitySheet>) -> AnyElement {
    sheet.clone().into_any_element()
}
