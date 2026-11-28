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
//! - [`requests`]: pending contact requests from the daemon (note, held
//!   messages, safety number, Accept / Decline / Block), the
//!   auto-accept setting, and this account's own requests on its contacts.

pub mod add_form;
pub mod my_card;
pub mod requests;
pub mod rows;

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::input::InputEvent;
use gpui_kit::component::v_flex;
use gpui_kit::prelude::*;
use gpui_kit::{Context, Entity, EventEmitter, IntoElement, Render, Subscription, Window, div};
use ramflux_desktop_core::{
    LockState, Participant, ParticipantId, Store, UiEventKind, methods, selectors,
};
use ramflux_desktop_navigation::scale::{space, text_caption};

use super::section;
use crate::app::{StoreEvent, StoreHandle};
use crate::bubble::EventSink;
use add_form::{AddForm, FormNote, Submission};
use my_card::{Copied, MyCard};

/// Whether the open account names no home node: contact cards (import
/// and export) need its home-node profile on the daemon.
pub fn without_home_node(store: &Store) -> bool {
    store.accounts().any(|a| a.lock_state == LockState::Unlocked && a.node_id.is_none())
}

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
    /// The contact whose safety number is being compared before it is
    /// marked verified.
    verifying: Option<ParticipantId>,
    /// Whether the add-contact dialog is open (the form moves from the
    /// panel into the dialog while it is).
    adding: bool,
    /// Tracked by the dialogs' focus trap.
    dialog_focus: gpui_kit::FocusHandle,
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
            verifying: None,
            adding: false,
            dialog_focus: cx.focus_handle(),
            _subscriptions: vec![subscription, enter],
        }
    }

    /// Opens the add-contact dialog and focuses its field.
    pub fn open_add_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.adding = true;
        self.form.focus(window, cx);
        cx.notify();
    }

    /// Whether the add-contact dialog is open.
    pub fn add_dialog_open(&self) -> bool {
        self.adding
    }

    /// Whether a dialog of this dock (add contact, compare safety numbers)
    /// is open.
    pub fn dialog_open(&self) -> bool {
        self.adding || self.verifying.is_some()
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
            let handle = self.store.read(cx);
            self.form.take_submission(handle.store(), handle.active_account(), cx)
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
            Some(Submission::Resolve(name)) => {
                self.sink.emit(UiEventKind::ResolveUsernameRequested { name }, window, cx);
            }
            Some(Submission::Username(name)) => {
                self.sink.emit(UiEventKind::AddContactByUsernameRequested { name }, window, cx);
            }
            Some(Submission::Card(card)) => {
                self.sink.emit(
                    UiEventKind::AddContactCardRequested { card, replace_provisional_root: None },
                    window,
                    cx,
                );
            }
            Some(Submission::CardReplacing { card, previous_root }) => {
                self.sink.emit(
                    UiEventKind::AddContactCardRequested {
                        card,
                        replace_provisional_root: Some(previous_root),
                    },
                    window,
                    cx,
                );
            }
            None => {}
        }
        cx.notify();
    }

    /// Replace pressed on a root to confirm.
    pub fn confirm_root(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let submission = self.form.confirm_root(self.store.read(cx).store());
        self.emit_submission(submission, window, cx);
    }

    /// Opens the safety number comparison of `participant` (Verify pressed).
    pub fn start_verify(&mut self, participant: ParticipantId, cx: &mut Context<Self>) {
        self.verifying = Some(participant);
        cx.notify();
    }

    /// The contact whose safety number is being compared, for tests.
    pub fn verifying(&self) -> Option<&ParticipantId> {
        self.verifying.as_ref()
    }

    /// The numbers matched: marks the contact verified.
    pub fn confirm_verify(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(participant_id) = self.verifying.take() {
            self.sink.emit(UiEventKind::VerifySafetyPressed { participant_id }, window, cx);
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
            self.adding = false;
            self.form.input().update(cx, |input, cx| input.set_value("", window, cx));
        }
    }
}

impl ContactsDock {
    /// The add-contact form with its handlers.
    fn add_form_element(
        &mut self,
        dialog: bool,
        cx: &mut Context<Self>,
    ) -> (gpui_kit::AnyElement, Option<gpui_kit::AnyElement>) {
        let handle = self.store.read(cx);
        let store: &Store = handle.store();
        let account = handle.active_account().cloned();
        let add_reason = crate::gate::reason(store, methods::CONTACT_ADD);
        let card_reason = crate::gate::reason(store, methods::CONTACT_CARD_IMPORT);
        let no_home = without_home_node(store);
        let controls = add_form::Controls {
            disabled: add_reason.or(card_reason),
            card_disabled: no_home.then(|| crate::i18n::tr("backend.contact.no_home_node")),
            on_add: cx.listener(|this, _, window, cx| this.submit_add(window, cx)),
            on_import: cx.listener(|this, _, window, cx| this.pick_card_file(window, cx)),
            on_root: add_form::RootHandlers {
                replace: Box::new(cx.listener(|this, _, window, cx| this.confirm_root(window, cx))),
                keep: Box::new(cx.listener(|this, _, _, cx| {
                    this.form.keep_root();
                    cx.notify();
                })),
            },
            lookup: self.form.shown_lookup(store, account.as_ref()).cloned(),
        };
        if dialog {
            let (body, button) = add_form::render_dialog(&self.form, controls, cx);
            (body, Some(button))
        } else {
            (add_form::render(&self.form, controls, cx), None)
        }
    }

    /// The open dialog of this dock, drawn by the workspace over the whole
    /// window: the safety number comparison, or add contact.
    pub fn dialog(&mut self, cx: &mut Context<Self>) -> Option<gpui_kit::AnyElement> {
        if let Some(id) = self.verifying.clone() {
            let (contact, reason) = {
                let store = self.store.read(cx).store();
                (
                    store.participant(&id)?.clone(),
                    crate::gate::reason(store, methods::SAFETY_VERIFY),
                )
            };
            let this = cx.weak_entity();
            let cancel = std::rc::Rc::new(move |_: &mut Window, cx: &mut gpui_kit::App| {
                this.update(cx, |dock, cx| {
                    dock.verifying = None;
                    cx.notify();
                })
                .ok();
            });
            let confirm =
                Box::new(cx.listener(|this, _, window, cx| this.confirm_verify(window, cx)));
            return Some(rows::comparison_dialog(
                &contact,
                confirm,
                cancel,
                reason.as_ref(),
                &self.dialog_focus,
                cx,
            ));
        }
        if !self.adding {
            return None;
        }
        let (form, add_button) = self.add_form_element(true, cx);
        let this = cx.weak_entity();
        let close = move |_: &mut Window, cx: &mut gpui_kit::App| {
            this.update(cx, |dock, cx| dock.close_add_dialog(cx)).ok();
        };
        let cancel = {
            let this = cx.weak_entity();
            move |_: &mut Window, cx: &mut gpui_kit::App| {
                this.update(cx, |dock, cx| dock.close_add_dialog(cx)).ok();
            }
        };
        Some(
            crate::modal::Modal::new(
                "contact-add-dialog",
                crate::i18n::tr("dock.contacts.add.title"),
                &self.dialog_focus,
                close,
            )
            .child(form)
            .button(crate::modal::cancel_button(
                "contact-add-cancel",
                crate::i18n::tr("dock.contacts.add.cancel"),
                cx,
                cancel,
            ))
            .button(add_button.unwrap_or_else(|| gpui_kit::div().into_any_element()))
            .render(cx),
        )
    }

    fn close_add_dialog(&mut self, cx: &mut Context<Self>) {
        self.adding = false;
        cx.notify();
    }
}

impl ContactsDock {
    /// The one contact the Profile panel shows: its row with the safety
    /// state, Verify, Message and the actions menu. Opened from a
    /// conversation, the panel is about that peer only; the global contact
    /// tools live in the Contacts section.
    pub fn peer_section(
        &mut self,
        peer: &ParticipantId,
        cx: &mut Context<Self>,
    ) -> Option<gpui_kit::AnyElement> {
        let handle = self.store.read(cx);
        let store: &Store = handle.store();
        let contact = store.participant(peer)?.clone();
        let verify_reason = crate::gate::reason(store, methods::SAFETY_VERIFY);
        let request = requests::outgoing(store, &contact);
        let id = contact.id.clone();
        let on_message = cx.listener(move |_, _, _, cx| {
            cx.emit(ContactsDockEvent::MessageRequested(id.clone()));
        });
        let id = contact.id.clone();
        let verify = rows::VerifyHandlers {
            reason: verify_reason,
            start: Box::new(cx.listener(move |this, _, window, cx| {
                this.start_verify(id.clone(), cx);
                window.focus(&this.dialog_focus, cx);
            })),
        };
        Some(rows::contact_row(&contact, &self.sink, verify, on_message, request, cx))
    }
}

impl Render for ContactsDock {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let handle = self.store.read(cx);
        let store: &Store = handle.store();
        // Answers to incoming links are local when requests cannot travel.
        let local_answers = crate::gate::reason(store, methods::CONTACT_REQUEST).is_some();
        let requests = rows::requests(store);
        let pending = selectors::pending_contact_requests(store);
        let request_reason = requests::reason(store);
        let has_link_requests = requests.iter().any(|r| matches!(r, rows::Request::Link(_)));
        let blocked: Vec<Participant> =
            selectors::blocked_contacts(store).into_iter().cloned().collect();
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
            .child(section("dock.contacts.card.title", cx))
            .child(card)
            // The heading stays once the setting is known, so the switch
            // reads as part of Requests, not of the card above.
            .when(
                !requests.is_empty()
                    || !pending.is_empty()
                    || store.request_policy().is_some()
                    || store.contact_requests_read_error().is_some(),
                |col| col.child(section("dock.contacts.requests", cx)),
            )
            .children(requests::read_error(store, cx))
            .children(requests::floors_notice(store, &self.sink, cx))
            .when(!requests.is_empty() || !pending.is_empty(), |col| {
                col.children(pending.iter().map(|r| {
                    requests::pending_row(
                        r,
                        store.participant(&r.peer),
                        &self.sink,
                        request_reason.as_ref(),
                        cx,
                    )
                }))
                .children(requests.iter().map(|r| rows::request_row(r, &self.sink, cx)))
                .when(has_link_requests && local_answers, |col| {
                    col.child(
                        div()
                            .id("contacts-requests-local")
                            .test_support()
                            .px(space(3.))
                            .pb(space(1.))
                            .text_size(text_caption())
                            .text_color(muted)
                            .child(crate::i18n::tr("dock.contacts.requests_local")),
                    )
                })
            })
            .when(store.request_policy().is_some(), |col| {
                col.child(requests::policy_switch(store, &self.sink, cx))
            })
            .when(!store.group_invitations().is_empty(), |col| {
                col.child(section("dock.contacts.invitations", cx)).children(
                    store
                        .group_invitations()
                        .iter()
                        .map(|invitation| requests::invitation_row(invitation, &self.sink, cx)),
                )
            })
            .when(!blocked.is_empty(), |col| {
                col.child(section("dock.contacts.blocked", cx))
                    .children(blocked.iter().map(|c| rows::blocked_row(c, &self.sink, cx)))
            })
    }
}
