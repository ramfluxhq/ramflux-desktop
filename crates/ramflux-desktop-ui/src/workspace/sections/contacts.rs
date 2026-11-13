// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The contact directory (design F02–F14, states F28–F30): this account's
//! contacts, searchable, each with its kind and safety state; the way into
//! the strangers area with its pending count; and Add contact.
//!
//! Choosing a contact shows its profile ([`super::contact_profile`]). A
//! blocked participant is not a contact and is listed only in the contacts
//! dock's blocked section.

use std::rc::Rc;

use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::InputEvent;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, Entity, EventEmitter, IntoElement, Render, SharedString, Subscription, Window,
    div,
};
use ramflux_desktop_core::{
    ListState, Listing, Participant, ParticipantId, ParticipantKind, Store, methods, selectors,
};
use ramflux_desktop_navigation::generated::catalog;

use super::directory::{self, DirectoryRow, Search, StateKeys};
use crate::actions::{NextConversation, PreviousConversation};
use crate::app::{StoreEvent, StoreHandle};
use crate::i18n::{self, tr, tr_args};
use crate::workspace::docks::contacts::rows::safety_label;

/// What the directory reports to the workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContactsDirectoryEvent {
    /// Show this contact's profile.
    Selected(ParticipantId),
    /// Open the add-contact field.
    AddContact,
    /// Open the strangers area.
    Strangers,
}

/// The contact directory's view state.
pub struct ContactsDirectory {
    store: Entity<StoreHandle>,
    search: Search,
    selected: Option<ParticipantId>,
    failure_mark: u64,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ContactsDirectoryEvent> for ContactsDirectory {}

/// Avatar shape of a participant kind.
pub(crate) fn avatar_kind(kind: ParticipantKind) -> catalog::AvatarKind {
    match kind {
        ParticipantKind::Human => catalog::AvatarKind::Person,
        _ => catalog::AvatarKind::Agent,
    }
}

impl ContactsDirectory {
    /// A directory over the rendered account's store.
    pub fn new(store: Entity<StoreHandle>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = Search::new("sections.contacts.search", window, cx);
        let subscriptions = vec![
            cx.subscribe(&store, |this, store, event: &StoreEvent, cx| {
                if let StoreEvent::Changed(changes) = event {
                    let store = store.read(cx).store();
                    if changes.is_reset()
                        || (changes.participants().next().is_some()
                            && store.failures_total() == this.failure_mark)
                    {
                        this.failure_mark = store.failures_total();
                    }
                    if changes.touches_docks() || changes.touches_status() {
                        cx.notify();
                    }
                }
            }),
            cx.subscribe(search.input(), |this, _, event: &InputEvent, cx| match event {
                InputEvent::Change => {
                    this.search.active = 0;
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => this.pick_active(cx),
                _ => {}
            }),
        ];
        let failure_mark = store.read(cx).store().failures_total();
        Self { store, search, selected: None, failure_mark, _subscriptions: subscriptions }
    }

    /// Marks the shown contact (from the workspace).
    pub fn set_selected(&mut self, id: Option<ParticipantId>, cx: &mut Context<Self>) {
        if self.selected != id {
            self.selected = id;
            cx.notify();
        }
    }

    /// Focuses the search field.
    pub fn focus_search(&self, window: &mut Window, cx: &mut App) {
        self.search.focus(window, cx);
    }

    fn entries<'a>(&self, store: &'a Store, cx: &App) -> Vec<&'a Participant> {
        let query = self.search.query(cx);
        let mut contacts: Vec<&Participant> = selectors::contacts(store)
            .into_iter()
            .filter(|p| directory::matches(&query, &[&p.display, p.id.as_str()]))
            .collect();
        contacts.sort_by_key(|p| p.display.to_lowercase());
        contacts
    }

    /// The contact ids in display order (keyboard and tests).
    pub fn entry_ids(&self, cx: &App) -> Vec<ParticipantId> {
        let store = self.store.read(cx).store();
        self.entries(store, cx).into_iter().map(|p| p.id.clone()).collect()
    }

    fn pick_active(&mut self, cx: &mut Context<Self>) {
        let ids = self.entry_ids(cx);
        if let Some(id) = ids.get(self.search.active.min(ids.len().saturating_sub(1))) {
            cx.emit(ContactsDirectoryEvent::Selected(id.clone()));
        }
    }

    fn step(&mut self, step: isize, cx: &mut Context<Self>) {
        let len = self.entry_ids(cx).len();
        self.search.step(step, len);
        cx.notify();
    }

    fn failure(&self, store: &Store) -> Option<SharedString> {
        store
            .failures_since(self.failure_mark)
            .iter()
            .rev()
            .find(|failure| failure.method == "contact.list")
            .map(|failure| i18n::backend_text(&failure.message))
    }
}

impl Render for ContactsDirectory {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (rows, list, status, failure, strangers, add_reason, filtered) = {
            let store = self.store.read(cx).store();
            let entries = self.entries(store, cx);
            let list = ListState::of(store.is_listed(Listing::Contacts), entries.is_empty());
            let rows: Vec<DirectoryRow> = entries
                .iter()
                .map(|p| DirectoryRow {
                    key: p.id.to_string().into(),
                    title: ramflux_desktop_navigation::row::list_text(&p.display),
                    subtitle: safety_label(p.safety),
                    avatar: avatar_kind(p.kind),
                    trailing: crate::identity::kind_text(p.kind_claim),
                    selected: self.selected.as_ref() == Some(&p.id),
                    checked: None,
                })
                .collect();
            let strangers = super::strangers::pending(store).len();
            (
                rows,
                list,
                store.source_status(),
                self.failure(store),
                strangers,
                crate::gate::reason(store, methods::CONTACT_ADD),
                !self.search.query(cx).is_empty(),
            )
        };
        let count = rows.len();
        let state = if filtered && list != ListState::Loading && rows.is_empty() {
            Some(
                div()
                    .id("contacts-no-match")
                    .test_support()
                    .px_3()
                    .py_2()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr("sections.no_match"))
                    .into_any_element(),
            )
        } else {
            directory::state(
                directory::StateSpec {
                    id: "contacts-state",
                    list,
                    status,
                    failure,
                    keys: StateKeys {
                        loading: "sections.contacts.loading",
                        offline: "sections.contacts.offline",
                        failed: "sections.contacts.failed",
                        empty: "sections.contacts.empty",
                        empty_help: "sections.contacts.empty_help",
                    },
                    action: None,
                },
                cx,
            )
        };
        let add = crate::gate::gated(
            Button::new("contacts-add")
                .small()
                .ghost()
                .icon(IconName::UserPlus)
                .accessibility_label(tr("sections.contacts.add"))
                .tooltip(tr("sections.contacts.add"))
                .on_click(cx.listener(|_, _, _, cx| cx.emit(ContactsDirectoryEvent::AddContact))),
            add_reason,
        );
        let strangers_label =
            tr_args("sections.strangers.entry", &[("count", &strangers.to_string())]);
        let this = cx.weak_entity();
        let pick: directory::RowAction = Rc::new(move |key, _, cx| {
            let id = ParticipantId::new(key);
            this.update(cx, |_, cx| cx.emit(ContactsDirectoryEvent::Selected(id))).ok();
        });
        v_flex()
            .id("contact-directory")
            .test_support()
            .key_context("ConversationList")
            .on_action(cx.listener(|this, _: &NextConversation, _, cx| this.step(1, cx)))
            .on_action(cx.listener(|this, _: &PreviousConversation, _, cx| this.step(-1, cx)))
            .size_full()
            .min_w_0()
            .bg(cx.theme().sidebar)
            .child(directory::header(
                tr_args("sections.contacts.title", &[("count", &count.to_string())]),
                h_flex().child(add),
                cx,
            ))
            .child(self.search.field("contacts-search", tr("sections.contacts.search")))
            .child(
                div().px_2().pb_2().child(
                    Button::new("contacts-strangers")
                        .ghost()
                        .small()
                        .w_full()
                        .icon(IconName::Inbox)
                        .label(strangers_label.clone())
                        .accessibility_label(strangers_label)
                        .on_click(
                            cx.listener(|_, _, _, cx| cx.emit(ContactsDirectoryEvent::Strangers)),
                        ),
                ),
            )
            .children(state)
            .child(directory::rows(
                directory::RowList {
                    id: "contacts-list",
                    label: tr("sections.contacts.list"),
                    rows,
                    active: self.search.active,
                    on_pick: pick,
                    on_check: None,
                },
                cx,
            ))
    }
}
