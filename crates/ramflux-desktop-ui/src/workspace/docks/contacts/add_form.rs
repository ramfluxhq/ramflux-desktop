// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The add-contact form: one field for what the contact shared (their
//! contact card, a link to it, or their principal id), an Add button, an
//! Import card file… button, and two lines under the field: what the typed
//! text is and how far it is trusted, then what happened.
//!
//! The form tells a card from an id with the core's own rule
//! (`classify_contact_input`) and checks either with the policy's check
//! (`check_add_contact_card`, `check_add_contact`) before emitting, so a
//! refusal is explained next to the field. A card is checked by the daemon
//! against its signature and pins the identity it names; an id is looked
//! up on the contact's node and stays unverified until safety numbers are
//! compared, which the line under the field says before anything is sent.
//!
//! Once submitted the form waits for the outcome without clearing the
//! field: the contact appearing in the list (for a card, any contact the
//! answer names) clears it; a failed add is shown under the field with the
//! daemon's reason. Editing the field forgets a pending submission. A card
//! file is read here (at most [`MAX_CONTACT_CARD_LEN`] bytes of text) and
//! submitted as it is.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Sizable as _, h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Entity, Role, SharedString, Window, div};
use ramflux_desktop_core::{
    ContactCardProblem, ContactIdProblem, ContactInput, ContactLink, IntentError,
    MAX_CONTACT_CARD_LEN, ParticipantId, Store, check_add_contact, check_add_contact_card,
    classify_contact_input, methods,
};

use crate::app::ChangeSet;
use crate::i18n::{self, tr, tr_args};

/// What the line under the field says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormNote {
    /// Waiting for the daemon's answer about this id.
    Adding(ParticipantId),
    /// Waiting for the daemon's answer about a contact card.
    AddingCard,
    /// The id or card was refused before or by the daemon.
    Error(SharedString),
}

/// What the form emits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Submission {
    /// Add by principal id.
    Id(ParticipantId),
    /// Add from a contact card.
    Card(String),
}

/// A submission waiting for its outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Pending {
    /// By id; done when that participant is a contact.
    Id(ParticipantId, u64),
    /// By card (typed, or from a file); done when the answer names a
    /// contact.
    Card(u64, bool),
}

/// View-local state of the form.
pub struct AddForm {
    input: Entity<InputState>,
    note: Option<FormNote>,
    pending: Option<Pending>,
    /// The field value the note belongs to; editing it clears the note.
    noted_value: String,
}

/// Whether `id` names an established or requested contact in `store`.
fn is_contact(store: &Store, id: &ParticipantId) -> bool {
    store
        .participant(id)
        .is_some_and(|p| matches!(p.link_state(), ContactLink::Accepted | ContactLink::Outgoing))
}

impl AddForm {
    /// A form with an empty field.
    pub fn new(window: &mut Window, cx: &mut App) -> Self {
        let input =
            i18n::localized_input("dock.contacts.add.placeholder", window, cx, |input| input);
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

    /// Checks the typed text. Returns what to emit, or `None` with the
    /// reason shown under the field.
    pub fn take_submission(&mut self, store: &Store, cx: &App) -> Option<Submission> {
        let value = self.input.read(cx).value().trim().to_owned();
        self.noted_value.clone_from(&value);
        self.submit(store, &value)
    }

    /// Checks card text read from a file and submits it; the field keeps
    /// what was typed.
    pub fn take_card_file(&mut self, store: &Store, text: &str, cx: &App) -> Option<Submission> {
        if self.pending.is_some() {
            return None;
        }
        self.noted_value = self.input.read(cx).value().trim().to_owned();
        match check_add_contact_card(text) {
            Ok(card) => {
                self.pending = Some(Pending::Card(store.failures_total(), true));
                self.note = Some(FormNote::AddingCard);
                Some(Submission::Card(card.to_owned()))
            }
            Err(error) => {
                self.note = Some(FormNote::Error(refusal(&error)));
                None
            }
        }
    }

    /// Shows why a card file could not be read.
    pub fn file_failed(&mut self, reason: SharedString) {
        self.note = Some(FormNote::Error(reason));
    }

    fn submit(&mut self, store: &Store, value: &str) -> Option<Submission> {
        if self.pending.is_some() {
            return None;
        }
        let checked = match classify_contact_input(value) {
            ContactInput::Card(card) => check_add_contact_card(card).map(|card| {
                (Pending::Card(store.failures_total(), false), Submission::Card(card.into()))
            }),
            ContactInput::PrincipalId(id) => check_add_contact(store, id)
                .map(|id| (Pending::Id(id.clone(), store.failures_total()), Submission::Id(id))),
        };
        match checked {
            Ok((pending, submission)) => {
                self.note = Some(match &submission {
                    Submission::Id(id) => FormNote::Adding(id.clone()),
                    Submission::Card(_) => FormNote::AddingCard,
                });
                self.pending = Some(pending);
                Some(submission)
            }
            Err(error) => {
                self.note = Some(FormNote::Error(refusal(&error)));
                None
            }
        }
    }

    /// Follows the store after a submission: the contact appeared (clear the
    /// field) or the add failed (show why). Returns whether the field must
    /// be cleared.
    pub fn observe(&mut self, store: &Store, changes: Option<&ChangeSet>, cx: &App) -> bool {
        let value = self.input.read(cx).value().trim().to_owned();
        if value != self.noted_value {
            // The user edited the field: what was said no longer applies.
            self.note = None;
            self.pending = None;
            self.noted_value = value;
            return false;
        }
        let (added, since, method, typed) = match self.pending.clone() {
            None => return false,
            Some(Pending::Id(id, since)) => {
                (is_contact(store, &id), since, methods::CONTACT_ADD, true)
            }
            Some(Pending::Card(since, from_file)) => {
                let named = changes
                    .is_some_and(|changes| changes.participants().any(|id| is_contact(store, id)));
                (named, since, methods::CONTACT_CARD_IMPORT, !from_file)
            }
        };
        if added {
            self.pending = None;
            self.note = None;
            if typed {
                self.noted_value.clear();
            }
            return typed;
        }
        if let Some(failure) = store.failures_since(since).iter().rev().find(|f| f.method == method)
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

    /// What the field holds now and how far it will be trusted, for the
    /// line under it.
    fn kind_hint(&self, cx: &App) -> SharedString {
        let value = self.input.read(cx).value().trim().to_owned();
        if value.is_empty() {
            return tr("dock.contacts.add.help");
        }
        match classify_contact_input(&value) {
            ContactInput::Card(_) => tr("dock.contacts.add.kind_card"),
            ContactInput::PrincipalId(_) => tr("dock.contacts.add.kind_id"),
        }
    }
}

/// Reads a contact card file: at most [`MAX_CONTACT_CARD_LEN`] bytes of
/// UTF-8 text.
///
/// # Errors
///
/// The translated reason.
pub fn read_card_file(path: &std::path::Path) -> Result<String, SharedString> {
    use std::io::Read as _;
    let file = std::fs::File::open(path)
        .map_err(|e| tr_args("dock.contacts.add.file_unreadable", &[("reason", &e.to_string())]))?;
    let mut bytes = Vec::new();
    let limit = u64::try_from(MAX_CONTACT_CARD_LEN).unwrap_or(u64::MAX).saturating_add(1);
    file.take(limit)
        .read_to_end(&mut bytes)
        .map_err(|e| tr_args("dock.contacts.add.file_unreadable", &[("reason", &e.to_string())]))?;
    if bytes.len() > MAX_CONTACT_CARD_LEN {
        return Err(tr("dock.contacts.add.error.card_too_long"));
    }
    String::from_utf8(bytes).map_err(|_| tr("dock.contacts.add.file_not_text"))
}

/// Why the policy refuses the id or card, for the line under the field.
fn refusal(error: &IntentError) -> SharedString {
    match error {
        IntentError::InvalidContactId(ContactIdProblem::Empty)
        | IntentError::InvalidContactCard(ContactCardProblem::Empty) => {
            tr("dock.contacts.add.error.empty")
        }
        IntentError::InvalidContactId(ContactIdProblem::Spaces) => {
            tr("dock.contacts.add.error.spaces")
        }
        IntentError::InvalidContactId(ContactIdProblem::TooLong) => {
            tr("dock.contacts.add.error.too_long")
        }
        IntentError::InvalidContactCard(ContactCardProblem::TooLong) => {
            tr("dock.contacts.add.error.card_too_long")
        }
        IntentError::InvalidContactCard(ContactCardProblem::Control) => {
            tr("dock.contacts.add.error.card_control")
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

/// The form's controls, wired by the dock.
pub struct Controls<A, I> {
    /// Why adding is unavailable, if it is.
    pub disabled: Option<SharedString>,
    /// Add pressed.
    pub on_add: A,
    /// Import card file… pressed.
    pub on_import: I,
}

/// The form: field and Add button, the trust line, the note line, and the
/// card file import.
pub fn render<A, I>(form: &AddForm, controls: Controls<A, I>, cx: &App) -> AnyElement
where
    A: Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static,
    I: Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static,
{
    let Controls { disabled, on_add, on_import } = controls;
    let button = Button::new("contact-add")
        .small()
        .outline()
        .icon(IconName::Plus)
        .label(tr("dock.contacts.add.submit"))
        .disabled(form.is_pending())
        .on_click(on_add);
    let import = Button::new("contact-import")
        .xsmall()
        .ghost()
        .icon(IconName::FolderOpen)
        .label(tr("dock.contacts.add.import"))
        .disabled(form.is_pending())
        .on_click(on_import);
    let note = form.note().map(|note| {
        let (text, color) = match note {
            FormNote::Adding(id) => (
                tr_args("dock.contacts.add.adding", &[("id", id.as_str())]),
                cx.theme().muted_foreground,
            ),
            FormNote::AddingCard => {
                (tr("dock.contacts.add.adding_card"), cx.theme().muted_foreground)
            }
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
    let hint = form.kind_hint(cx);
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
                .child(crate::gate::gated(button, disabled.clone())),
        )
        .child(
            div()
                .id("contact-add-kind")
                .aria_label(hint.clone())
                .test_support()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(hint),
        )
        .children(note)
        .child(h_flex().child(crate::gate::gated(import, disabled)))
        .into_any_element()
}
