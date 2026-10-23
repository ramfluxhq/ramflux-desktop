// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The add-contact form: one field for the id the contact shared, an Add
//! button, and one line under the field for what happened.
//!
//! The form checks the id with the policy's own check before emitting
//! (`check_add_contact`), so a refusal is explained next to the field. Once
//! submitted it waits for the outcome without clearing the field: the
//! contact appearing in the list clears it; a failed `contact.add` is shown
//! under the field with the daemon's reason. Editing the field forgets a
//! pending submission.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Sizable as _, h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Entity, Role, SharedString, Window, div};
use ramflux_desktop_core::{
    ContactIdProblem, ContactLink, IntentError, ParticipantId, Store, check_add_contact, methods,
};

use crate::i18n::{self, tr, tr_args};

/// What the line under the field says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormNote {
    /// Waiting for the daemon's answer about this id.
    Adding(ParticipantId),
    /// The id was refused before or by the daemon.
    Error(SharedString),
}

/// View-local state of the form.
pub struct AddForm {
    input: Entity<InputState>,
    note: Option<FormNote>,
    /// The id submitted and the store's failure count at that time.
    pending: Option<(ParticipantId, u64)>,
    /// The field value the note belongs to; editing it clears the note.
    noted_value: String,
}

impl AddForm {
    /// A form with an empty field.
    pub fn new(window: &mut Window, cx: &mut App) -> Self {
        let placeholder = tr("dock.contacts.add.placeholder");
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        Self { input, note: None, pending: None, noted_value: String::new() }
    }

    /// The field.
    pub fn input(&self) -> &Entity<InputState> {
        &self.input
    }

    /// The line under the field.
    pub fn note(&self) -> Option<&FormNote> {
        self.note.as_ref()
    }

    /// Moves keyboard focus to the field.
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.input.update(cx, |input, cx| input.focus(window, cx));
    }

    /// Checks the typed id. Returns the id to emit, or `None` with the
    /// reason shown under the field.
    pub fn take_submission(&mut self, store: &Store, cx: &App) -> Option<ParticipantId> {
        if self.pending.is_some() {
            return None;
        }
        let value = self.input.read(cx).value().trim().to_owned();
        self.noted_value.clone_from(&value);
        match check_add_contact(store, &value) {
            Ok(id) => {
                self.pending = Some((id.clone(), store.failures_total()));
                self.note = Some(FormNote::Adding(id.clone()));
                Some(id)
            }
            Err(error) => {
                self.note = Some(FormNote::Error(refusal(&error)));
                None
            }
        }
    }

    /// Follows the store after a submission: the contact appeared (clear the
    /// field) or `contact.add` failed (show why). Returns whether the field
    /// must be cleared.
    pub fn observe(&mut self, store: &Store, cx: &App) -> bool {
        let value = self.input.read(cx).value().trim().to_owned();
        if value != self.noted_value {
            // The user edited the field: what was said no longer applies.
            self.note = None;
            self.pending = None;
            self.noted_value = value;
            return false;
        }
        let Some((id, since)) = self.pending.clone() else { return false };
        let added = store.participant(&id).is_some_and(|p| {
            matches!(p.link_state(), ContactLink::Accepted | ContactLink::Outgoing)
        });
        if added {
            self.pending = None;
            self.note = None;
            self.noted_value.clear();
            return true;
        }
        if let Some(failure) =
            store.failures_since(since).iter().rev().find(|f| f.method == methods::CONTACT_ADD)
        {
            self.pending = None;
            self.note = Some(FormNote::Error(tr_args(
                "dock.contacts.add.failed",
                &[("reason", &i18n::backend_text(&failure.message))],
            )));
        }
        false
    }

    /// Whether a submission waits for its outcome.
    pub fn is_pending(&self) -> bool {
        self.pending.is_some()
    }
}

/// Why the policy refuses the id, for the line under the field.
fn refusal(error: &IntentError) -> SharedString {
    match error {
        IntentError::InvalidContactId(ContactIdProblem::Empty) => {
            tr("dock.contacts.add.error.empty")
        }
        IntentError::InvalidContactId(ContactIdProblem::Spaces) => {
            tr("dock.contacts.add.error.spaces")
        }
        IntentError::InvalidContactId(ContactIdProblem::TooLong) => {
            tr("dock.contacts.add.error.too_long")
        }
        IntentError::ContactLinkState { link: ContactLink::Blocked, .. } => {
            tr("dock.contacts.add.error.blocked")
        }
        IntentError::ContactLinkState { link: ContactLink::Incoming, .. } => {
            tr("dock.contacts.add.error.incoming")
        }
        IntentError::ContactLinkState { .. } => tr("dock.contacts.add.error.exists"),
        other => SharedString::from(other.to_string()),
    }
}

/// The form: label, field and Add button, help, and the note line.
pub fn render(
    form: &AddForm,
    disabled: Option<SharedString>,
    on_add: impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let button = Button::new("contact-add")
        .small()
        .outline()
        .icon(IconName::Plus)
        .label(tr("dock.contacts.add.submit"))
        .disabled(form.is_pending())
        .on_click(on_add);
    let note = form.note().map(|note| {
        let (text, color) = match note {
            FormNote::Adding(id) => (
                tr_args("dock.contacts.add.adding", &[("id", id.as_str())]),
                cx.theme().muted_foreground,
            ),
            FormNote::Error(text) => (text.clone(), cx.theme().danger),
        };
        div()
            .id("contact-add-note")
            .role(Role::Status)
            .aria_label(text.clone())
            .test_support()
            .text_xs()
            .text_color(color)
            .child(text)
    });
    v_flex()
        .id("contact-add")
        .gap_1()
        .px_3()
        .pb_2()
        .child(
            h_flex()
                .gap_2()
                .child(
                    Input::new(form.input())
                        .id("contact-add-id")
                        .aria_label(tr("dock.contacts.add.label"))
                        .small()
                        .flex_1(),
                )
                .child(crate::gate::gated(button, disabled)),
        )
        .children(note)
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(tr("dock.contacts.add.help")),
        )
        .into_any_element()
}
