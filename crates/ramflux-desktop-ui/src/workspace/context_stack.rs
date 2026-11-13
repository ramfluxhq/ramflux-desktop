// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Full account-scoped conversation list. Store changes own its snapshot;
//! the retained input owns search, and the Kit virtual list owns scrolling.

use crate::actions::{NextConversation, PreviousConversation};
use crate::app::{ChangeSet, StoreEvent, StoreHandle};
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
    Context, Entity, EventEmitter, Focusable, IntoElement, Pixels, Render, ScrollStrategy,
    SharedString, Size, Subscription, Window, div, px, size,
};
use ramflux_desktop_core::projection::conversations::{
    ConversationRow, Preview, is_visible, row, strangers,
};
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
    at: Option<ramflux_desktop_core::Timestamp>,
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
    search: super::context_search::Index,
    store: Entity<StoreHandle>,
    current: Option<ContextId>,
    query: Entity<InputState>,
    rows: Rc<Vec<Rc<Entry>>>,
    filter: ListFilter,
    active: usize,
    sizes: Rc<Vec<Size<Pixels>>>,
    row_height: Pixels,
    projected: usize,
    query_invalid: bool,
    /// The "Unread" chip of the message list (design board 02).
    unread_only: bool,
    scroll: VirtualListScrollHandle,
    _subscriptions: Vec<Subscription>,
}
impl EventEmitter<ContextStackEvent> for ContextStack {}

impl ContextStack {
    /// Creates the search input once and subscribes to the active account store.
    pub fn new(store: Entity<StoreHandle>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // The placeholder follows a language switch like every other field.
        let query = i18n::localized_input("navigation.search", window, cx, |input| input);
        let subscriptions = vec![
            cx.subscribe_in(&store, window, |this, _, event: &StoreEvent, window, cx| {
                if let StoreEvent::Changed(changes) = event {
                    if changes.is_reset() {
                        this.query.update(cx, |input, cx| input.set_value("", window, cx));
                    }
                    if !changes.is_domain_empty() {
                        this.update_rows(changes, cx);
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
        let mut search = super::context_search::Index::default();
        search.reset(store.read(cx).store());
        let mut list = Self {
            search,
            store,
            current: None,
            query,
            rows: Rc::new(Vec::new()),
            filter: ListFilter::Messages,
            active: 0,
            sizes: Rc::new(Vec::new()),
            row_height: window.rem_size() * theme::ROW_REM,
            projected: 0,
            query_invalid: false,
            unread_only: false,
            scroll: VirtualListScrollHandle::new(),
            _subscriptions: subscriptions,
        };
        list.refresh(cx);
        list
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let active = self.rows.get(self.active).map(|row| row.id.clone());
        let handle = self.store.read(cx);
        self.query_invalid = self.query.read(cx).value().len()
            > ramflux_desktop_core::projection::conversations::QUERY_LIMIT;
        let mut rows =
            entries(handle.store(), &self.query.read(cx).value(), self.filter, &self.search);
        if self.unread_only && self.filter == ListFilter::Messages {
            rows.retain(|row| row.unread > 0);
        }
        self.rows = Rc::new(rows);
        self.projected += self.rows.len();
        self.finish_refresh(active, cx);
    }

    fn update_rows(&mut self, changes: &ChangeSet, cx: &mut Context<Self>) {
        if self.unread_only {
            // The unread view is small and its membership follows every
            // read change, so it is rebuilt.
            let store = self.store.read(cx).store();
            self.search.reset(store);
            self.refresh(cx);
            return;
        }
        let store = self.store.read(cx).store();
        let Some(ids) = changes.directory_contexts(store) else {
            self.search.reset(store);
            self.refresh(cx);
            return;
        };
        self.search.prune(store);
        self.search.apply(store, changes);
        if ids.is_empty() {
            cx.notify();
            return;
        }
        let active = self.rows.get(self.active).map(|row| row.id.clone());
        let query = self.query.read(cx).value().trim().to_lowercase();
        let rows = Rc::make_mut(&mut self.rows);
        for id in ids {
            rows.retain(|entry| entry.id != id);
            if self.query_invalid || !is_visible(store) {
                continue;
            }
            let Some(context) = store.context(&id) else {
                continue;
            };
            if !self.search.matches(&id, &query) {
                continue;
            }
            if let Some(entry) =
                entry(store, row(store, context, context.last_activity), self.filter)
            {
                self.projected += 1;
                let ix = rows
                    .binary_search_by(|existing| order(existing, &entry))
                    .unwrap_or_else(|ix| ix);
                rows.insert(ix, Rc::new(entry));
            }
        }
        self.finish_refresh(active, cx);
    }

    fn finish_refresh(&mut self, active: Option<ContextId>, cx: &mut Context<Self>) {
        self.active = active
            .and_then(|id| self.rows.iter().position(|row| row.id == id))
            .unwrap_or(self.active)
            .min(self.rows.len().saturating_sub(1));
        if self.sizes.len() != self.rows.len() {
            self.sizes = Rc::new(vec![size(px(0.), self.row_height); self.rows.len()]);
        }
        cx.notify();
    }

    /// Number of row projections performed since this list was created.
    /// Used to check that a source delta does not rebuild untouched rows.
    pub fn projected_rows(&self) -> usize {
        self.projected
    }

    /// Number of source documents normalized; typing a query does not change it.
    pub fn search_builds(&self) -> usize {
        self.search.builds
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
    /// Shows every conversation, or only those with unread messages.
    pub fn set_unread_only(&mut self, unread_only: bool, cx: &mut Context<Self>) {
        if self.unread_only != unread_only {
            self.unread_only = unread_only;
            self.active = 0;
            self.refresh(cx);
        }
    }

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

    fn move_cursor(&mut self, next: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.rows.is_empty() {
            return;
        }
        self.active = if next {
            (self.active + 1).min(self.rows.len() - 1)
        } else {
            self.active.saturating_sub(1)
        };
        self.scroll.scroll_to_item(self.active, ScrollStrategy::Center);
        self.focus_search(window, cx);
        cx.notify();
    }
}

fn entries(
    store: &Store,
    query: &str,
    filter: ListFilter,
    search: &super::context_search::Index,
) -> Vec<Rc<Entry>> {
    if query.len() > ramflux_desktop_core::projection::conversations::QUERY_LIMIT
        || !is_visible(store)
    {
        return Vec::new();
    }
    let query = query.trim().to_lowercase();
    let mut entries: Vec<_> = store
        .contexts()
        .filter(|context| search.matches(&context.id, &query))
        .filter_map(|context| {
            entry(store, row(store, context, context.last_activity), filter).map(Rc::new)
        })
        .collect();
    entries.sort_by(|a, b| order(a, b));
    entries
}

fn order(a: &Entry, b: &Entry) -> std::cmp::Ordering {
    b.pinned.cmp(&a.pinned).then_with(|| b.at.cmp(&a.at)).then_with(|| a.id.cmp(&b.id))
}

fn entry(store: &Store, row: ConversationRow<'_>, filter: ListFilter) -> Option<Entry> {
    if filter == ListFilter::Groups && row.context.kind != ContextKind::Group {
        return None;
    }
    let has_task = || {
        store
            .context_moments(&row.context.id)
            .any(|m| matches!(m.kind, MomentKind::TaskProgress(_) | MomentKind::TaskEvent(_)))
    };
    // The directory has no task count or history-complete fact. Keep unknown rows
    // visible rather than asserting that an unloaded conversation has no tasks.
    let task_unknown = filter == ListFilter::Tasks && !has_task();
    let preview = if task_unknown {
        tr(if store.context_moment_count(&row.context.id) == 0 {
            "navigation.tasks_not_loaded"
        } else {
            "navigation.tasks_none_loaded"
        })
    } else {
        match row.preview {
            Preview::Text(text) => ramflux_desktop_navigation::row::list_text(text),
            Preview::Deleted => tr("moment.message.deleted"),
            Preview::Unavailable => tr("navigation.not_loaded"),
            Preview::Attachment => tr("moment.message.attachment"),
            Preview::Activity => tr("navigation.activity"),
            Preview::Empty => tr("navigation.empty_preview"),
        }
    };
    let agent = row.context.kind == ContextKind::Direct
        && row
            .context
            .participants
            .iter()
            .any(|id| store.participant(id).is_some_and(|p| p.kind == ParticipantKind::Agent));
    Some(Entry {
        id: row.context.id.clone(),
        at: row.at,
        title: ramflux_desktop_navigation::row::list_text(&row.title),
        preview,
        time: row.at.map_or_else(SharedString::default, |at| i18n::time_of_day(at).into()),
        kind: if agent { AvatarKind::Agent } else { AvatarKind::Person },
        pinned: row.context.pinned,
        muted: row.context.muted,
        needs_me: row.attention,
        unread: row.context.unread,
    })
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
    crate::i18n::clauses(&text)
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
        let palette = ramflux_desktop_navigation::palette(cx);
        let row_height = window.rem_size() * theme::ROW_REM;
        if self.row_height != row_height {
            self.row_height = row_height;
            self.sizes = Rc::new(vec![size(px(0.), row_height); self.rows.len()]);
        }
        let sizes = self.sizes.clone();
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
                                .px_3()
                                .rounded(gpui_kit::px(
                                    ramflux_desktop_navigation::palette(cx).radius_sm,
                                ))
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
            .on_action(cx.listener(|this, _: &NextConversation, window, cx| {
                this.move_cursor(true, window, cx)
            }))
            .on_action(cx.listener(|this, _: &PreviousConversation, window, cx| {
                this.move_cursor(false, window, cx)
            }))
            .bg(palette.nav)
            .border_r_1()
            .border_color(palette.line)
            .px_3()
            .pt(gpui_kit::rems(24.0 / 15.0))
            .gap_4()
            // Design board 02: the list title with its add command, the
            // search field, then the strangers entry as the first row.
            .child(
                h_flex()
                    .px_2()
                    .h(gpui_kit::rems(28.0 / 15.0))
                    .child(
                        div()
                            .flex_1()
                            .text_size(gpui_kit::rems(palette.size_title / 15.0))
                            .font_semibold()
                            .child(tr(match self.filter {
                                ListFilter::Messages => "navigation.messages",
                                ListFilter::Groups => "navigation.groups",
                                ListFilter::Tasks => "navigation.tasks",
                            })),
                    )
                    .child(new_menu),
            )
            .child(
                Input::new(&self.query)
                    .id("conversation-search")
                    .small()
                    .prefix(Icon::new(IconName::Search)),
            )
            .when(self.filter == ListFilter::Messages, |pane| {
                let chip = |id: &'static str, key: &'static str, on: bool, unread: bool| {
                    Button::new(id)
                        .ghost()
                        .xsmall()
                        .rounded(gpui_kit::px(palette.radius_chip))
                        .selected(on)
                        .toggled(on)
                        .label(tr(key))
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.set_unread_only(unread, cx)),
                        )
                };
                pane.child(
                    h_flex()
                        .id("stack-filters")
                        .gap_1()
                        .child(chip(
                            "stack-filter-all",
                            "stack.filter.all",
                            !self.unread_only,
                            false,
                        ))
                        .child(chip(
                            "stack-filter-unread",
                            "stack.filter.unread",
                            self.unread_only,
                            true,
                        )),
                )
            })
            // The strangers entry belongs to the message list only (boards 02, E01).
            .when(self.filter == ListFilter::Messages, |pane| {
                pane.child({
                    let mut row = Row::new(
                        tr("navigation.strangers"),
                        tr_args(
                            "navigation.strangers_detail",
                            &[("count", &request_count.to_string())],
                        ),
                    );
                    row.glyph = Some("?".into());
                    row.unread = u32::try_from(request_count).unwrap_or(u32::MAX);
                    Button::new("strangers-entry")
                        .ghost()
                        .w_full()
                        .h(gpui_kit::rems(theme::ROW_REM))
                        .px_3()
                        .rounded(gpui_kit::px(palette.radius_sm))
                        .accessibility_label(tr_args(
                            "navigation.strangers_count",
                            &[("count", &request_count.to_string())],
                        ))
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(ContextStackEvent::Strangers)))
                        .child(row.content(cx))
                })
            })
            .when(self.filter == ListFilter::Tasks, |pane| {
                pane.child(
                    div()
                        .id("tasks-history-notice")
                        .role(gpui_kit::Role::Status)
                        .test_support()
                        .px_3()
                        .pb_2()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(tr("navigation.tasks_history_notice")),
                )
            })
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
