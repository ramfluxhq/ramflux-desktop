// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The group directory (design C05): the groups this account has joined,
//! searchable, each with its member count and this account's role, and the
//! New group command (C01, C30). Choosing a group shows its conversation
//! with the member panel (C02–C04, C24); the workspace owns that.
//!
//! States follow C32–C35: loading until the source listed the account's
//! conversations, offline (cached groups stay listed), failed (a group read
//! failed after the last success), and no groups yet.

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
    ContextId, ContextKind, ListState, Listing, Role, Store, methods, selectors,
};
use ramflux_desktop_navigation::generated::catalog;

use super::directory::{self, DirectoryRow, Search, StateKeys};
use crate::actions::{NextConversation, PreviousConversation};
use crate::app::{StoreEvent, StoreHandle};
use crate::i18n::{self, tr, tr_args};

/// What the directory reports to the workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupDirectoryEvent {
    /// Show this group.
    Selected(ContextId),
    /// Open the new-group form.
    Create,
}

/// The group directory's view state: search, cursor, the shown group and
/// the failure mark (a group failure older than the mark is stale).
pub struct GroupDirectory {
    store: Entity<StoreHandle>,
    search: Search,
    selected: Option<ContextId>,
    failure_mark: u64,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<GroupDirectoryEvent> for GroupDirectory {}

/// One listed group.
struct GroupEntry {
    id: ContextId,
    title: String,
    members: usize,
    role: Option<Role>,
    unread: u32,
}

impl GroupDirectory {
    /// A directory over the rendered account's store.
    pub fn new(store: Entity<StoreHandle>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = Search::new("sections.groups.search", window, cx);
        let subscriptions = vec![
            cx.subscribe(&store, |this, store, event: &StoreEvent, cx| {
                if let StoreEvent::Changed(changes) = event {
                    let store = store.read(cx).store();
                    // A reset is an account switch: its failures are not ours.
                    // Fresh contexts without a new failure mean a read succeeded.
                    if changes.is_reset()
                        || (changes.contexts().next().is_some()
                            && store.failures_total() == this.failure_mark)
                    {
                        this.failure_mark = store.failures_total();
                    }
                    if changes.touches_context_stack() || changes.touches_status() {
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

    /// Marks the shown group (from the workspace).
    pub fn set_selected(&mut self, id: Option<ContextId>, cx: &mut Context<Self>) {
        if self.selected != id {
            self.selected = id;
            cx.notify();
        }
    }

    /// Focuses the search field.
    pub fn focus_search(&self, window: &mut Window, cx: &mut App) {
        self.search.focus(window, cx);
    }

    /// The listed groups in display order (most recent activity first).
    fn entries(&self, store: &Store, cx: &App) -> Vec<GroupEntry> {
        let query = self.search.query(cx);
        let me = selectors::self_participant(store);
        let mut entries: Vec<GroupEntry> = store
            .contexts()
            .filter(|context| context.kind == ContextKind::Group)
            .filter_map(|context| {
                let title = selectors::context_title(store, context);
                directory::matches(&query, &[&title]).then(|| GroupEntry {
                    id: context.id.clone(),
                    members: context.participants.len(),
                    role: me.as_ref().map(|me| context.role_of(me)),
                    unread: context.unread,
                    title,
                })
            })
            .collect();
        entries.sort_by(|a, b| {
            let at = |id: &ContextId| store.context(id).and_then(|c| c.last_activity);
            at(&b.id).cmp(&at(&a.id)).then_with(|| a.title.cmp(&b.title))
        });
        entries
    }

    /// The ids in display order (keyboard and tests).
    pub fn entry_ids(&self, cx: &App) -> Vec<ContextId> {
        let store = self.store.read(cx).store();
        self.entries(store, cx).into_iter().map(|entry| entry.id).collect()
    }

    fn pick_active(&mut self, cx: &mut Context<Self>) {
        let ids = self.entry_ids(cx);
        if let Some(id) = ids.get(self.search.active.min(ids.len().saturating_sub(1))) {
            cx.emit(GroupDirectoryEvent::Selected(id.clone()));
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
            .find(|failure| failure.method.starts_with("group."))
            .map(|failure| i18n::backend_text(&failure.message))
    }
}

impl Render for GroupDirectory {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let store = self.store.read(cx).store();
        let entries = self.entries(store, cx);
        let list = ListState::of(store.is_listed(Listing::Contexts), entries.is_empty());
        let filtered = !self.search.query(cx).is_empty();
        let create_reason = crate::gate::reason(store, methods::GROUP_CREATE);
        let create = crate::gate::gated(
            Button::new("groups-create")
                .small()
                .ghost()
                .icon(IconName::Plus)
                .accessibility_label(tr("sections.groups.create"))
                .tooltip(tr("sections.groups.create"))
                .on_click(cx.listener(|_, _, _, cx| cx.emit(GroupDirectoryEvent::Create))),
            create_reason,
        );
        let count = entries.len();
        let state = if filtered && list != ListState::Loading && entries.is_empty() {
            Some(
                div()
                    .id("groups-no-match")
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
                    id: "groups-state",
                    list,
                    status: store.source_status(),
                    failure: self.failure(store),
                    keys: StateKeys {
                        loading: "sections.groups.loading",
                        offline: "sections.groups.offline",
                        failed: "sections.groups.failed",
                        empty: "sections.groups.empty",
                        empty_help: "sections.groups.empty_help",
                    },
                    action: None,
                },
                cx,
            )
        };
        let rows: Vec<DirectoryRow> = entries
            .into_iter()
            .map(|entry| {
                let members =
                    tr_args("sections.groups.members", &[("count", &entry.members.to_string())]);
                let subtitle = match entry.role {
                    Some(role) => i18n::join(&members, &i18n::role(role)),
                    None => members,
                };
                DirectoryRow {
                    key: entry.id.to_string().into(),
                    title: ramflux_desktop_navigation::row::list_text(&entry.title),
                    subtitle,
                    avatar: catalog::AvatarKind::Group,
                    trailing: (entry.unread > 0).then(|| entry.unread.to_string().into()),
                    selected: self.selected.as_ref() == Some(&entry.id),
                    checked: None,
                }
            })
            .collect();
        let this = cx.weak_entity();
        let pick: directory::RowAction = Rc::new(move |key, _, cx| {
            let id = ContextId::new(key);
            this.update(cx, |_, cx| cx.emit(GroupDirectoryEvent::Selected(id))).ok();
        });
        v_flex()
            .id("group-directory")
            .test_support()
            .key_context("ConversationList")
            .on_action(cx.listener(|this, _: &NextConversation, _, cx| this.step(1, cx)))
            .on_action(cx.listener(|this, _: &PreviousConversation, _, cx| this.step(-1, cx)))
            .size_full()
            .min_w_0()
            .bg(ramflux_desktop_navigation::palette(cx).nav)
            .border_r_1()
            .border_color(ramflux_desktop_navigation::palette(cx).line)
            .px_3()
            .pt(gpui_kit::rems(24.0 / 15.0))
            .gap_4()
            .child(directory::header(
                tr_args("sections.groups.title", &[("count", &count.to_string())]),
                h_flex().child(create),
                cx,
            ))
            .child(self.search.field("groups-search", tr("sections.groups.search"), cx))
            .children(state)
            .child(directory::rows(
                directory::RowList {
                    id: "groups-list",
                    label: tr("sections.groups.list"),
                    rows,
                    active: self.search.active,
                    on_pick: pick,
                    on_check: None,
                },
                cx,
            ))
            .child(
                div()
                    .px_3()
                    .py_2()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr("sections.groups.joined_only")),
            )
    }
}
