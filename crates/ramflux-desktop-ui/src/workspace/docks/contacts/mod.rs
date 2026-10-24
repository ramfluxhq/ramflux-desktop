// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The contacts dock: the add-contact form (a contact card, a link to one,
//! or a principal id), this account's own contact card to share, open
//! contact requests (answer in place), every contact with its safety state,
//! reachability and kind, safety number, a Verify action while unverified,
//! Message (reported to the workspace as
//! [`ContactsDockEvent::MessageRequested`]) and a Block / Remove menu, then
//! blocked participants (Unblock, Remove). Key changes stay visible here
//! until verified.
//!
//! - [`add_form`]: the add-contact form and its view-local state.
//! - [`my_card`]: this account's contact card (QR code, text, copy).
//! - [`rows`]: request, contact and blocked rows.

pub mod add_form;
pub mod my_card;
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
use add_form::{AddForm, FormNote, Submission};
use my_card::{Copied, MyCard};

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
    card: MyCard,
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
            if let StoreEvent::Changed(changes) = event {
                this.observe(Some(changes), window, cx);
                cx.notify();
            }
        });
        let form = AddForm::new(window, cx);
        let enter =
            cx.subscribe_in(form.input(), window, |this, _, event, window, cx| match event {
                InputEvent::PressEnter { .. } => this.submit_add(window, cx),
                InputEvent::Change => {
                    this.observe(None, window, cx);
                    cx.notify();
                }
                _ => {}
            });
        Self {
            store,
            sink,
            form,
            card: MyCard::default(),
            _subscriptions: vec![subscription, enter],
        }
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
        let submission = {
            let store = self.store.read(cx).store();
            self.form.take_submission(store, cx)
        };
        self.emit_submission(submission, window, cx);
    }

    fn emit_submission(
        &mut self,
        submission: Option<Submission>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match submission {
            Some(Submission::Id(participant_id)) => {
                self.sink.emit(UiEventKind::AddContactRequested { participant_id }, window, cx);
            }
            Some(Submission::Card(card)) => {
                self.sink.emit(UiEventKind::AddContactCardRequested { card }, window, cx);
            }
            None => {}
        }
        cx.notify();
    }

    /// Adds the contact card in the file at `path` (what the Import card
    /// file… dialog chose).
    pub fn import_card_file(
        &mut self,
        path: &std::path::Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let submission = match add_form::read_card_file(path) {
            Ok(text) => {
                let store = self.store.read(cx).store();
                self.form.take_card_file(store, &text, cx)
            }
            Err(reason) => {
                self.form.file_failed(reason);
                None
            }
        };
        self.emit_submission(submission, window, cx);
    }

    fn pick_card_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.weak_entity();
        crate::file_dialog::open_file(
            crate::i18n::tr("dock.contacts.add.import_prompt"),
            window,
            cx,
            move |path, window, cx| {
                this.update(cx, |dock, cx| {
                    dock.import_card_file(std::path::Path::new(&path), window, cx);
                })
                .ok();
            },
        );
    }

    /// Unfolds this account's contact card and asks the daemon for it.
    pub fn show_card(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.card.show(self.store.read(cx).store());
        self.sink.emit(UiEventKind::ShareContactCardRequested, window, cx);
        cx.notify();
    }

    /// What the contact card section shows, for tests.
    pub fn card_view(&self, cx: &gpui_kit::App) -> my_card::CardView {
        self.card.view(self.store.read(cx).store())
    }

    fn observe(
        &mut self,
        changes: Option<&crate::app::ChangeSet>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let clear = {
            let store = self.store.read(cx).store();
            self.form.observe(store, changes, cx)
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
        let card_reason = crate::gate::reason(store, methods::CONTACT_CARD_IMPORT);
        let form = add_form::render(
            &self.form,
            add_form::Controls {
                disabled: add_reason.or(card_reason),
                on_add: cx.listener(|this, _, window, cx| this.submit_add(window, cx)),
                on_import: cx.listener(|this, _, window, cx| this.pick_card_file(window, cx)),
            },
            cx,
        );
        let this = cx.weak_entity();
        let card = my_card::render(
            &self.card,
            store,
            my_card::Handlers {
                show: Box::new(cx.listener(|this, _, window, cx| this.show_card(window, cx))),
                hide: Box::new(cx.listener(|this, _, _, cx| {
                    this.card.hide();
                    cx.notify();
                })),
                copy: std::rc::Rc::new(
                    move |part: Copied, text: String, _: &mut Window, cx: &mut gpui_kit::App| {
                        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(text));
                        this.update(cx, |dock, cx| {
                            dock.card.copied(part);
                            cx.notify();
                        })
                        .ok();
                    },
                ),
            },
            cx,
        );
        let muted = cx.theme().muted_foreground;
        v_flex()
            .id("dock-contacts")
            .size_full()
            .overflow_y_scroll()
            .child(section("dock.contacts.add.title", cx))
            .child(form)
            .child(section("dock.contacts.card.title", cx))
            .child(card)
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
