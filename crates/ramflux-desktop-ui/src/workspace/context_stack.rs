// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Full account-scoped conversation list. Store changes own its snapshot;
//! the retained input owns search, and the Kit virtual list owns scrolling.

use crate::actions::{NextConversation, PreviousConversation};
use crate::app::{StoreEvent, StoreHandle};
use crate::i18n::{self, tr, tr_args};
use crate::ids::domain_id;
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Selectable as _, Sizable as _, StyledExt as _,
    VirtualListScrollHandle, h_flex, v_flex, v_virtual_list,
};
use gpui_kit::prelude::*;
use gpui_kit::{
    Context, Entity, EventEmitter, Focusable, IntoElement, Render, ScrollStrategy, SharedString,
    Subscription, Window, div, px, size,
};
use ramflux_desktop_core::projection::conversations::{Preview, conversations, strangers};
use ramflux_desktop_core::{
    ContactLink, ContextId, ContextKind, Listing, MomentKind, ParticipantId, ParticipantKind,
    Store, selectors,
};
use ramflux_desktop_navigation::{
    row::{AvatarKind, Row},
    theme,
};
use std::rc::Rc;

/// Reported to the workspace; navigation never submits business effects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextStackEvent {
    /// Select an existing conversation.
    Selected(ContextId),
    /// Start a conversation with a contact.
    NewConversation(ParticipantId),
    /// Open the add-contact field.
    AddContact,
    /// Open the new-group form.
    NewGroup,
    /// Open incoming requests without accepting anybody.
    Strangers,
}

/// Which source conversations are shown by a workspace section.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ListFilter {
    /// Every conversation, including unpinned ones.
    #[default]
    Messages,
    /// Groups only.
    Groups,
    /// Conversations with existing task moments.
    Tasks,
}

struct Entry {
    id: ContextId,
    title: SharedString,
    preview: SharedString,
    time: SharedString,
    kind: AvatarKind,
    pinned: bool,
    muted: bool,
    needs_me: usize,
    unread: u32,
}

/// Retained full-list state. The historical name keeps keyboard/API callers stable.
pub struct ContextStack {
    store: Entity<StoreHandle>,
    current: Option<ContextId>,
    query: Entity<InputState>,
    rows: Rc<Vec<Entry>>,
    filter: ListFilter,
    active: usize,
    query_invalid: bool,
    scroll: VirtualListScrollHandle,
    _subscriptions: Vec<Subscription>,
}
impl EventEmitter<ContextStackEvent> for ContextStack {}

impl ContextStack {
    /// Creates the search input once and subscribes to the active account store.
    pub fn new(store: Entity<StoreHandle>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| InputState::new(window, cx).placeholder(tr("navigation.search")));
        let subscriptions = vec![
            cx.subscribe_in(&store, window, |this, _, event: &StoreEvent, window, cx| {
                if let StoreEvent::Changed(changes) = event {
                    if changes.is_reset() {
                        this.query.update(cx, |input, cx| input.set_value("", window, cx));
                    }
                    if !changes.is_domain_empty() {
                        this.refresh(cx);
                    }
                }
            }),
            cx.subscribe_in(&query, window, |this, _, event: &InputEvent, _, cx| match event {
                InputEvent::Change => {
                    this.active = 0;
                    this.refresh(cx);
                }
                InputEvent::PressEnter { .. } => this.select_active(cx),
                _ => {}
            }),
        ];
        let mut list = Self {
            store,
            current: None,
            query,
            rows: Rc::new(Vec::new()),
            filter: ListFilter::Messages,
            active: 0,
            query_invalid: false,
            scroll: VirtualListScrollHandle::new(),
            _subscriptions: subscriptions,
        };
        list.refresh(cx);
        list
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let handle = self.store.read(cx);
        self.query_invalid = self.query.read(cx).value().len()
            > ramflux_desktop_core::projection::conversations::QUERY_LIMIT;
        self.rows = Rc::new(entries(handle.store(), &self.query.read(cx).value(), self.filter));
        self.active = self.active.min(self.rows.len().saturating_sub(1));
        cx.notify();
    }

    /// Marks selection without changing unread state or submitting a request.
    pub fn set_current(&mut self, current: Option<ContextId>, cx: &mut Context<Self>) {
        self.current = current;
        if let Some(ix) = self.rows.iter().position(|row| Some(&row.id) == self.current.as_ref()) {
            self.active = ix;
        }
        cx.notify();
    }

    /// Selects a navigation section and resets its local scroll/keyboard cursor.
    pub fn set_filter(&mut self, filter: ListFilter, cx: &mut Context<Self>) {
        self.filter = filter;
        self.active = 0;
        self.refresh(cx);
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
    }

    /// Focuses the retained search field from the window command.
    pub fn focus_search(&self, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self.query.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    }

    /// Current visible IDs in the same order as keyboard navigation.
    pub fn entry_ids(&self, _: &Store) -> Vec<ContextId> {
        self.rows.iter().map(|row| row.id.clone()).collect()
    }

    fn select_active(&mut self, cx: &mut Context<Self>) {
        if let Some(row) = self.rows.get(self.active) {
            cx.emit(ContextStackEvent::Selected(row.id.clone()));
        }
    }

    fn move_cursor(&mut self, next: bool, cx: &mut Context<Self>) {
        if self.rows.is_empty() {
            return;
        }
        self.active = if next {
            (self.active + 1).min(self.rows.len() - 1)
        } else {
            self.active.saturating_sub(1)
        };
        self.scroll.scroll_to_item(self.active, ScrollStrategy::Top);
        cx.notify();
    }
}

fn entries(store: &Store, query: &str, filter: ListFilter) -> Vec<Entry> {
    let mut rows = Vec::new();
    let mut offset = 0;
    loop {
        let Ok(page) = conversations(store, query, offset, 100) else {
            return rows;
        };
        for row in page.rows {
            if filter == ListFilter::Groups && row.context.kind != ContextKind::Group {
                continue;
            }
            if filter == ListFilter::Tasks
                && !store.context_moments(&row.context.id).any(|m| {
                    matches!(m.kind, MomentKind::TaskProgress(_) | MomentKind::TaskEvent(_))
                })
            {
                continue;
            }
            let preview = match row.preview {
                Preview::Text(text) => text.to_owned().into(),
                Preview::Deleted => tr("moment.message.deleted"),
                Preview::Unavailable => tr("navigation.not_loaded"),
                Preview::Attachment => tr("moment.message.attachment"),
                Preview::Activity => tr("navigation.activity"),
                Preview::Empty => tr("navigation.empty_preview"),
            };
            let agent = row.context.kind == ContextKind::Direct
                && row.context.participants.iter().any(|id| {
                    store.participant(id).is_some_and(|p| p.kind == ParticipantKind::Agent)
                });
            rows.push(Entry {
                id: row.context.id.clone(),
                title: row.title.into(),
                preview,
                time: row.at.map_or_else(SharedString::default, |at| i18n::time_of_day(at).into()),
                kind: if agent { AvatarKind::Agent } else { AvatarKind::Person },
                pinned: row.context.pinned,
                muted: row.context.muted,
                needs_me: row.attention,
                unread: row.context.unread,
            });
        }
        match page.next_offset {
            Some(next) => offset = next,
            None => break,
        }
    }
    rows
}

fn accessible_name(entry: &Entry) -> SharedString {
    let mut text = vec![entry.title.to_string(), entry.preview.to_string()];
    if entry.pinned {
        text.push(tr("stack.pinned").to_string());
    }
    if entry.muted {
        text.push(tr("stack.muted").to_string());
    }
    if entry.unread > 0 {
        text.push(tr_args("stack.unread", &[("count", &entry.unread.to_string())]).to_string());
    }
    if entry.needs_me > 0 {
        text.push(tr_args("stack.needs_me", &[("count", &entry.needs_me.to_string())]).to_string());
    }
    text.join(", ").into()
}

impl Render for ContextStack {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let store = self.store.read(cx).store();
        let (request_count, _) = strangers(store);
        let placeholder = if self.rows.is_empty() && !self.query_invalid {
            crate::list_state::placeholder(
                "stack-list-state",
                crate::list_state::of(store, Listing::Contexts, true),
                if self.query.read(cx).value().is_empty() {
                    "stack.empty"
                } else {
                    "navigation.no_matches"
                },
                cx,
            )
        } else {
            None
        };
        let contacts: Vec<(ParticipantId, String)> = selectors::contacts(store)
            .into_iter()
            .filter(|p| p.link_state() == ContactLink::Accepted)
            .map(|p| (p.id.clone(), p.display.clone()))
            .collect();
        let this = cx.weak_entity();
        let new_menu = Button::new("stack-new")
            .ghost()
            .xsmall()
            .icon(IconName::Plus)
            .accessibility_label(tr("stack.new"))
            .tooltip(tr("stack.new"))
            .dropdown_menu(move |menu, _, _| {
                let emit = |event: ContextStackEvent| {
                    let this = this.clone();
                    move |_: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut gpui_kit::App| {
                        this.update(cx, |_, cx| cx.emit(event.clone())).ok();
                    }
                };
                let menu = contacts.iter().fold(menu, |menu, (id, name)| {
                    menu.item(
                        PopupMenuItem::new(tr_args("stack.new_with", &[("name", name)]))
                            .on_click(emit(ContextStackEvent::NewConversation(id.clone()))),
                    )
                });
                let menu = if contacts.is_empty() {
                    menu.item(PopupMenuItem::new(tr("stack.new_no_contacts")).disabled(true))
                } else {
                    menu
                };
                menu.separator()
                    .item(
                        PopupMenuItem::new(tr("stack.new_group"))
                            .on_click(emit(ContextStackEvent::NewGroup)),
                    )
                    .item(
                        PopupMenuItem::new(tr("stack.add_contact"))
                            .on_click(emit(ContextStackEvent::AddContact)),
                    )
            });
        let row_height = window.rem_size() * theme::ROW_REM;
        let sizes = Rc::new(vec![size(px(0.), row_height); self.rows.len()]);
        let list =
            v_virtual_list(cx.entity(), "conversation-rows", sizes, |this, range, window, cx| {
                range
                    .filter_map(|ix| {
                        let entry = this.rows.get(ix)?;
                        let id = entry.id.clone();
                        let selected = this.current.as_ref() == Some(&id);
                        let mut content = Row::new(entry.title.clone(), entry.preview.clone());
                        content.time = entry.time.clone();
                        content.unread = entry.unread;
                        content.attention = entry.needs_me;
                        content.kind = entry.kind;
                        Some(
                            Button::new(domain_id("stack", &id))
                                .ghost()
                                .w_full()
                                .h(gpui_kit::rems(theme::ROW_REM))
                                .selected(selected)
                                .accessibility_label(accessible_name(entry))
                                .when(
                                    ix == this.active
                                        && this.query.read(cx).focus_handle(cx).is_focused(window),
                                    |button| button.border_1().border_color(cx.theme().ring),
                                )
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    cx.emit(ContextStackEvent::Selected(id.clone()))
                                }))
                                .child(content.content(cx))
                                .into_any_element(),
                        )
                    })
                    .collect()
            })
            .track_scroll(&self.scroll);
        v_flex()
            .id("context-stack")
            .key_context("ConversationList")
            .h_full()
            .w_full()
            .min_w_0()
            .min_h_0()
            .on_action(cx.listener(|this, _: &NextConversation, _, cx| this.move_cursor(true, cx)))
            .on_action(
                cx.listener(|this, _: &PreviousConversation, _, cx| this.move_cursor(false, cx)),
            )
            .bg(cx.theme().sidebar)
            .border_r_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .p_3()
                    .child(div().flex_1().font_medium().child(tr(match self.filter {
                        ListFilter::Messages => "navigation.messages",
                        ListFilter::Groups => "navigation.groups",
                        ListFilter::Tasks => "navigation.tasks",
                    })))
                    .child(new_menu),
            )
            .child(
                div().px_3().pb_2().child(
                    Input::new(&self.query)
                        .id("conversation-search")
                        .small()
                        .prefix(Icon::new(IconName::Search)),
                ),
            )
            .child(
                div().px_2().pb_2().child(
                    Button::new("strangers-entry")
                        .ghost()
                        .w_full()
                        .label(tr_args(
                            "navigation.strangers_count",
                            &[("count", &request_count.to_string())],
                        ))
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(ContextStackEvent::Strangers))),
                ),
            )
            .children(placeholder)
            .when(self.query_invalid, |pane| {
                pane.child(
                    div()
                        .id("conversation-query-error")
                        .test_support()
                        .role(gpui_kit::Role::Status)
                        .aria_label(tr("navigation.query_error"))
                        .px_3()
                        .pb_2()
                        .text_sm()
                        .text_color(cx.theme().danger)
                        .child(tr("navigation.query_error")),
                )
            })
            .child(div().flex_1().min_h_0().child(list))
    }
}
