// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The contacts dock: the add-contact form, open contact requests (answer
//! in place), every contact with its safety state, safety number, a Verify
//! action while unverified, Message (reported to the workspace as
//! [`ContactsDockEvent::MessageRequested`]) and a Block / Remove menu, then
//! blocked participants (Unblock, Remove). Key changes stay visible here
//! until verified.
//!
//! - [`add_form`]: the add-contact form and its view-local state.
//! - [`rows`]: request, contact and blocked rows.

pub mod add_form;
pub mod rows;

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::input::InputEvent;
use gpui_kit::component::v_flex;
use gpui_kit::prelude::*;
use gpui_kit::{Context, Entity, EventEmitter, IntoElement, Render, Subscription, Window, div};
use ramflux_desktop_core::{
    Listing, Participant, ParticipantId, Store, UiEventKind, methods, selectors,
};

use super::section;
use crate::app::{StoreEvent, StoreHandle};
use crate::bubble::EventSink;
use add_form::{AddForm, FormNote};

/// Reported to the workspace (navigation, not intents).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContactsDockEvent {
    /// Message pressed on a contact.
    MessageRequested(ParticipantId),
}

/// The contacts dock content.
pub struct ContactsDock {
    store: Entity<StoreHandle>,
    sink: EventSink,
    form: AddForm,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ContactsDockEvent> for ContactsDock {}

impl ContactsDock {
    /// A dock over `store`, emitting through `sink` (scoped to the dock).
    pub fn new(
        store: Entity<StoreHandle>,
        sink: EventSink,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscription = cx.subscribe_in(&store, window, |this, _, event, window, cx| {
            if let StoreEvent::Changed(_) = event {
                this.observe(window, cx);
                cx.notify();
            }
        });
        let form = AddForm::new(window, cx);
        let enter =
            cx.subscribe_in(form.input(), window, |this, _, event, window, cx| match event {
                InputEvent::PressEnter { .. } => this.submit_add(window, cx),
                InputEvent::Change => {
                    this.observe(window, cx);
                    cx.notify();
                }
                _ => {}
            });
        Self { store, sink, form, _subscriptions: vec![subscription, enter] }
    }

    /// Moves keyboard focus to the add-contact field.
    pub fn focus_add(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.form.focus(window, cx);
    }

    /// The line under the add-contact field, for tests.
    pub fn add_note(&self) -> Option<&FormNote> {
        self.form.note()
    }

    /// The add-contact field's text, for tests.
    pub fn add_value(&self, cx: &gpui_kit::App) -> String {
        self.form.input().read(cx).value().to_string()
    }

    /// Whether the add-contact field has keyboard focus.
    pub fn add_focused(&self, window: &Window, cx: &gpui_kit::App) -> bool {
        gpui_kit::Focusable::focus_handle(self.form.input().read(cx), cx).is_focused(window)
    }

    /// Submits the add-contact field.
    pub fn submit_add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let id = {
            let store = self.store.read(cx).store();
            self.form.take_submission(store, cx)
        };
        if let Some(participant_id) = id {
            self.sink.emit(UiEventKind::AddContactRequested { participant_id }, window, cx);
        }
        cx.notify();
    }

    fn observe(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let clear = {
            let store = self.store.read(cx).store();
            self.form.observe(store, cx)
        };
        if clear {
            self.form.input().update(cx, |input, cx| input.set_value("", window, cx));
        }
    }
}

impl Render for ContactsDock {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let store: &Store = self.store.read(cx).store();
        let verify_reason = crate::gate::reason(store, methods::SAFETY_VERIFY);
        let add_reason = crate::gate::reason(store, methods::CONTACT_ADD);
        // Answers to incoming links are local when requests cannot travel.
        let local_answers = crate::gate::reason(store, methods::CONTACT_REQUEST).is_some();
        let requests = rows::requests(store);
        let has_link_requests = requests.iter().any(|r| matches!(r, rows::Request::Link(_)));
        let contacts: Vec<Participant> = selectors::contacts(store).into_iter().cloned().collect();
        let blocked: Vec<Participant> =
            selectors::blocked_contacts(store).into_iter().cloned().collect();
        let form = add_form::render(
            &self.form,
            add_reason,
            cx.listener(|this, _, window, cx| this.submit_add(window, cx)),
            cx,
        );
        let muted = cx.theme().muted_foreground;
        v_flex()
            .id("dock-contacts")
            .size_full()
            .overflow_y_scroll()
            .child(section("dock.contacts.add.title", cx))
            .child(form)
            .when(!requests.is_empty(), |col| {
                col.child(section("dock.contacts.requests", cx))
                    .children(requests.iter().map(|r| rows::request_row(r, &self.sink, cx)))
                    .when(has_link_requests && local_answers, |col| {
                        col.child(
                            div()
                                .id("contacts-requests-local")
                                .test_support()
                                .px_3()
                                .pb_1()
                                .text_xs()
                                .text_color(muted)
                                .child(crate::i18n::tr("dock.contacts.requests_local")),
                        )
                    })
            })
            .child(section("dock.contacts.list", cx))
            .children(crate::list_state::placeholder(
                "contacts-list-state",
                crate::list_state::of(store, Listing::Contacts, contacts.is_empty()),
                "dock.contacts.empty",
                cx,
            ))
            .children(contacts.iter().map(|c| {
                let id = c.id.clone();
                let on_message = cx.listener(move |_, _, _, cx| {
                    cx.emit(ContactsDockEvent::MessageRequested(id.clone()));
                });
                rows::contact_row(c, &self.sink, verify_reason.as_ref(), on_message, cx)
            }))
            .when(!blocked.is_empty(), |col| {
                col.child(section("dock.contacts.blocked", cx))
                    .children(blocked.iter().map(|c| rows::blocked_row(c, &self.sink, cx)))
            })
    }
}
