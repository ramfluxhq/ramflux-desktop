// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The contact directory (design F02–F14, states F28–F30): this account's
//! contacts, searchable, each with its kind and safety state; the way into
//! the strangers area with its pending count; and Add contact.
//!
//! Choosing a contact shows its profile ([`super::contact_profile`]). A
//! blocked participant is not a contact and is listed only in the contacts
//! dock's blocked section.

use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::StyledExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::InputEvent;
use gpui_kit::component::{Selectable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Context, Entity, EventEmitter, IntoElement, Render, SharedString,
    Subscription, Window, div,
};
use ramflux_desktop_core::{
    ListState, Listing, Participant, ParticipantId, ParticipantKind, RequestAnswer, Store,
    UiEventKind, methods, selectors,
};
use ramflux_desktop_navigation::generated::catalog;
use ramflux_desktop_sdk::business::ContractGap;

use super::directory::{self, Search, StateKeys};
use crate::actions::{NextConversation, PreviousConversation};
use crate::app::{StoreEvent, StoreHandle};
use crate::i18n::{self, tr, tr_args};
use crate::style::DesignButton as _;
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
    /// Show the people table (no contact chosen).
    People,
    /// Open the groups section.
    Groups,
}

/// The contact directory's view state.
pub struct ContactsDirectory {
    store: Entity<StoreHandle>,
    sink: crate::bubble::EventSink,
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
    pub fn new(
        store: Entity<StoreHandle>,
        sink: crate::bubble::EventSink,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
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
        Self { store, sink, search, selected: None, failure_mark, _subscriptions: subscriptions }
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

impl ContactsDirectory {
    /// The people table shown in the detail pane while no contact is chosen
    /// (design board 07): pending strangers with their answers, then every
    /// contact with "View profile".
    pub fn detail(
        &mut self,
        tools: Option<AnyElement>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = ramflux_desktop_navigation::palette(cx);
        let cursor_row = self.search.cursor(window, cx);
        let store = self.store.read(cx).store();
        let entries = self.entries(store, cx);
        let list = ListState::of(store.is_listed(Listing::Contacts), entries.is_empty());
        let filtered = !self.search.query(cx).is_empty();
        let rows: Vec<(ParticipantId, SharedString, SharedString, catalog::AvatarKind)> = entries
            .iter()
            .map(|p| {
                (
                    p.id.clone(),
                    ramflux_desktop_navigation::row::list_text(&p.display),
                    match crate::identity::kind_text(p.kind_claim) {
                        Some(kind) => i18n::join(&kind, &safety_label(p.safety)),
                        None => safety_label(p.safety),
                    },
                    avatar_kind(p.kind),
                )
            })
            .collect();
        let requests: Vec<(String, SharedString, SharedString)> = super::strangers::pending(store)
            .into_iter()
            .map(|request| {
                let name = store
                    .participant(&request.peer)
                    .map_or_else(|| request.peer.short(), |p| p.display.clone());
                // Board 07: "@handle · Identity not verified · note".
                let unverified = store
                    .participant(&request.peer)
                    .is_none_or(|p| p.safety != ramflux_desktop_core::SafetyState::Verified);
                let mut note = super::stranger_detail::handle(&request.peer);
                if unverified {
                    note = i18n::join(&note, &tr("sections.strangers.unverified_chip"));
                }
                if let Some(text) = request.note.as_deref().or(request.preview.as_deref()) {
                    note = i18n::join(&note, &ramflux_desktop_navigation::row::list_text(text));
                }
                (request.id.clone(), ramflux_desktop_navigation::row::list_text(&name), note)
            })
            .collect();
        let state = if filtered && list != ListState::Loading && rows.is_empty() {
            Some(
                div()
                    .id("contacts-no-match")
                    .test_support()
                    .py_2()
                    .text_sm()
                    .text_color(palette.secondary)
                    .child(tr("sections.no_match"))
                    .into_any_element(),
            )
        } else {
            directory::state(
                directory::StateSpec {
                    id: "contacts-state",
                    list,
                    status: store.source_status(),
                    failure: self.failure(store),
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
        let add_reason = crate::gate::reason(store, methods::CONTACT_ADD);
        let rem = |px: f32| gpui_kit::rems(px / 15.0);
        let avatar = |initial: SharedString, kind: catalog::AvatarKind| {
            let agent = matches!(kind, catalog::AvatarKind::Agent);
            div()
                .flex_none()
                .size(rem(40.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(if agent { rem(10.0) } else { rem(palette.radius_full) })
                .bg(if agent { palette.agent_av } else { palette.avatar_person })
                .text_color(if agent { palette.on_agent_av } else { palette.on_avatar_person })
                .font_semibold()
                .child(initial)
        };
        let section_title =
            |text: SharedString| div().pt_2().text_size(rem(palette.size_subheading)).child(text);
        let answer_reason = crate::workspace::docks::contacts::requests::reason(store);
        let strangers = (!requests.is_empty()).then(|| {
            v_flex()
                .id("contacts-strangers-table")
                .test_support()
                .gap_3()
                .child(section_title(tr_args(
                    "sections.contacts.strangers_count",
                    &[("count", &requests.len().to_string())],
                )))
                .children(requests.into_iter().map(|(id, name, note)| {
                    let answer = |suffix: &str, answer: RequestAnswer, label: &str| {
                        let sink = self.sink.clone();
                        let request_id = id.clone();
                        let button = Button::new(SharedString::from(format!(
                            "contacts-request-{id}-{suffix}"
                        )))
                        .label(tr(label))
                        .accessibility_label(tr_args(
                            &format!("{label}_for"),
                            &[("name", name.as_ref())],
                        ))
                        .on_click(move |_, window, cx| {
                            sink.emit(
                                UiEventKind::ContactRequestAnswered {
                                    request_id: request_id.clone(),
                                    answer,
                                },
                                window,
                                cx,
                            );
                        });
                        let button = match answer {
                            RequestAnswer::Accept => button.design_primary(cx),
                            _ => button.ghost().h(rem(32.0)).px(rem(12.0)),
                        };
                        crate::gate::gated(button, answer_reason.clone())
                    };
                    h_flex()
                        .id(SharedString::from(format!("contacts-request-{id}")))
                        .gap_4()
                        .p(rem(20.0))
                        .rounded(rem(palette.radius_card))
                        .bg(palette.nav)
                        .child(avatar(
                            ramflux_desktop_navigation::row::initial(&name),
                            catalog::AvatarKind::Person,
                        ))
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .child(div().text_size(rem(palette.size_body)).child(name.clone()))
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(rem(palette.size_secondary))
                                        .text_color(palette.secondary)
                                        .child(note),
                                ),
                        )
                        .child(answer(
                            "decline",
                            RequestAnswer::Decline,
                            "dock.contacts.request.decline",
                        ))
                        .child(crate::gate::gated(
                            Button::new(SharedString::from(format!("contacts-request-{id}-reply")))
                                .design_secondary(cx)
                                .label(tr("sections.strangers.reply")),
                            Some(directory::unavailable(ContractGap::StrangerActions)),
                        ))
                        .child(answer(
                            "accept",
                            RequestAnswer::Accept,
                            "dock.contacts.request.accept",
                        ))
                }))
        });
        let table = v_flex()
            .id("contacts-table")
            .test_support()
            .role(gpui_kit::Role::List)
            .aria_label(tr("sections.contacts.list"))
            .children(rows.into_iter().take(directory::ROW_LIMIT).enumerate().map(
                |(ix, (id, name, subtitle, kind))| {
                    let pick = id.clone();
                    // The search field's Up/Down cursor (Enter opens the profile).
                    let cursor = cursor_row == Some(ix);
                    h_flex()
                        .id(SharedString::from(format!("contacts-table-{id}")))
                        .role(gpui_kit::Role::ListItem)
                        .aria_selected(cursor)
                        .test_support()
                        .gap_4()
                        .px_2()
                        .rounded(rem(palette.radius_sm))
                        .when(cursor, |row| row.bg(palette.selected))
                        .py(rem(20.0))
                        .border_b_1()
                        .border_color(palette.line)
                        .child(avatar(ramflux_desktop_navigation::row::initial(&name), kind))
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .child(div().text_size(rem(palette.size_body)).child(name.clone()))
                                .child(
                                    div()
                                        .text_size(rem(palette.size_secondary))
                                        .text_color(palette.secondary)
                                        .child(subtitle),
                                ),
                        )
                        .child(
                            Button::new(SharedString::from(format!("contacts-view-{id}")))
                                .small()
                                .secondary()
                                .label(tr("sections.contacts.view_profile"))
                                .accessibility_label(tr_args(
                                    "sections.contacts.view_profile_for",
                                    &[("name", name.as_ref())],
                                ))
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    cx.emit(ContactsDirectoryEvent::Selected(pick.clone()))
                                })),
                        )
                },
            ));
        v_flex()
            .id("contacts-people")
            .test_support()
            .size_full()
            // The generated page header with the board's semibold title.
            .child(catalog::page_header_content(
                catalog::RowContent::new(
                    "contacts-people-header".into(),
                    tr("sections.contacts.people"),
                    tr("sections.contacts.people_help"),
                )
                .title_size(palette.size_subheading)
                .title_weight(gpui_kit::FontWeight::SEMIBOLD)
                .trailing(
                    h_flex().gap_3().children(tools).child(crate::gate::gated(
                        Button::new("contacts-add-detail")
                            .small()
                            .ghost()
                            .label(tr("sections.contacts.add"))
                            .on_click(cx.listener(|_, _, _, cx| {
                                cx.emit(ContactsDirectoryEvent::AddContact)
                            })),
                        add_reason,
                    )),
                ),
                false,
                palette,
            ))
            .child(
                v_flex()
                    .id("contacts-people-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(rem(32.0))
                    .py(rem(24.0))
                    .gap_4()
                    .children(strangers)
                    .child(section_title(tr("sections.contacts.all")))
                    .children(state)
                    .child(table),
            )
            .into_any_element()
    }
}

impl Render for ContactsDirectory {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Design board 07: the list pane holds the categories (people,
        // strangers, groups); the people table is the detail pane.
        let (contacts, strangers, groups, add_reason, people) = {
            let store = self.store.read(cx).store();
            (
                selectors::contacts(store).len(),
                super::strangers::pending(store).len(),
                store
                    .contexts()
                    .filter(|c| c.kind == ramflux_desktop_core::ContextKind::Group)
                    .count(),
                crate::gate::reason(store, methods::CONTACT_ADD),
                self.selected.is_none(),
            )
        };
        let palette = ramflux_desktop_navigation::palette(cx);
        let add = crate::gate::gated(
            Button::new("contacts-add")
                .small()
                .ghost()
                .icon(IconName::Plus)
                .accessibility_label(tr("sections.contacts.add"))
                .tooltip(tr("sections.contacts.add"))
                .on_click(cx.listener(|_, _, _, cx| cx.emit(ContactsDirectoryEvent::AddContact))),
            add_reason,
        );
        let category = |id: &'static str,
                        key: &'static str,
                        detail: SharedString,
                        selected: bool,
                        event: ContactsDirectoryEvent| {
            let row = ramflux_desktop_navigation::row::Row::new(tr(key), detail);
            Button::new(id)
                .ghost()
                .w_full()
                .h(gpui_kit::rems(ramflux_desktop_navigation::theme::ROW_REM))
                .px_3()
                .rounded(gpui_kit::px(palette.radius_sm))
                .selected(selected)
                .toggled(selected)
                .accessibility_label(tr(key))
                .on_click(cx.listener(move |_, _, _, cx| cx.emit(event.clone())))
                .child(
                    v_flex()
                        .w_full()
                        .child(
                            div()
                                .text_size(gpui_kit::rems(palette.size_body / 15.0))
                                .font_medium()
                                .child(row.title),
                        )
                        .child(
                            div()
                                .text_size(gpui_kit::rems(palette.size_secondary / 15.0))
                                .text_color(palette.secondary)
                                .child(row.preview),
                        ),
                )
        };
        v_flex()
            .id("contact-directory")
            .test_support()
            .key_context("ConversationList")
            .on_action(cx.listener(|this, _: &NextConversation, _, cx| this.step(1, cx)))
            .on_action(cx.listener(|this, _: &PreviousConversation, _, cx| this.step(-1, cx)))
            .size_full()
            .min_w_0()
            .bg(palette.nav)
            .border_r_1()
            .border_color(palette.line)
            .px_3()
            .pt(gpui_kit::rems(24.0 / 15.0))
            .gap_4()
            .child(directory::header(tr("navigation.contacts"), h_flex().child(add), cx))
            .child(self.search.field("contacts-search", tr("sections.contacts.search"), cx))
            .child(
                v_flex()
                    .gap_1()
                    .child(category(
                        "contacts-people-entry",
                        "sections.contacts.people",
                        tr_args(
                            "sections.contacts.people_count",
                            &[("count", &contacts.to_string())],
                        ),
                        people,
                        ContactsDirectoryEvent::People,
                    ))
                    .child(category(
                        "contacts-strangers",
                        "navigation.strangers",
                        tr_args(
                            "navigation.strangers_detail",
                            &[("count", &strangers.to_string())],
                        ),
                        false,
                        ContactsDirectoryEvent::Strangers,
                    ))
                    .child(category(
                        "contacts-groups-entry",
                        "navigation.groups",
                        tr_args(
                            "sections.contacts.groups_count",
                            &[("count", &groups.to_string())],
                        ),
                        false,
                        ContactsDirectoryEvent::Groups,
                    )),
            )
    }
}
