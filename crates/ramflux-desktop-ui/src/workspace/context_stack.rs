// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The compact context stack: pinned contexts, then contexts that need the
//! user (selector `context_stack`, at most six). It is a spatial switcher,
//! not a list of every conversation; other contexts are reached through the
//! "switch context" menu. Choosing a context is navigation (view-local), so it
//! is reported to the workspace as a [`ContextStackEvent`], not a UI event.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Selectable as _, Sizable as _, h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{
    Context, Entity, EventEmitter, IntoElement, Render, SharedString, Subscription, Window, div,
};
use ramflux_desktop_core::{ContextId, ContextKind, Lifecycle, Store, selectors};

use crate::app::{StoreEvent, StoreHandle};
use crate::i18n::{tr, tr_args};
use crate::ids::domain_id;

/// Reported to the workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextStackEvent {
    /// The user chose a context.
    Selected(ContextId),
}

/// The context stack view.
pub struct ContextStack {
    store: Entity<StoreHandle>,
    current: Option<ContextId>,
    _subscription: Subscription,
}

impl EventEmitter<ContextStackEvent> for ContextStack {}

impl ContextStack {
    /// A stack reading `store`; it re-renders only when contexts, pins or
    /// attention change.
    pub fn new(store: Entity<StoreHandle>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.subscribe(&store, |_, _, event: &StoreEvent, cx| {
            if let StoreEvent::Changed(changes) = event
                && changes.touches_context_stack()
            {
                cx.notify();
            }
        });
        Self { store, current: None, _subscription: subscription }
    }

    /// Marks the current context.
    pub fn set_current(&mut self, current: Option<ContextId>, cx: &mut Context<Self>) {
        if self.current != current {
            self.current = current;
            cx.notify();
        }
    }

    fn select(&mut self, id: ContextId, cx: &mut Context<Self>) {
        cx.emit(ContextStackEvent::Selected(id));
    }
}

struct Entry {
    id: ContextId,
    title: SharedString,
    kind: ContextKind,
    pinned: bool,
    needs_me: usize,
    unread: u32,
}

fn entries(store: &Store, current: Option<&ContextId>) -> Vec<Entry> {
    let mut ids = selectors::context_stack(store);
    if let Some(current) = current
        && !ids.contains(current)
        && store.context(current).is_some()
    {
        ids.push(current.clone());
    }
    ids.into_iter()
        .filter_map(|id| {
            let context = store.context(&id)?;
            let needs_me = store
                .moments()
                .filter(|m| m.context_id == id && m.lifecycle == Lifecycle::NeedsMe)
                .count();
            Some(Entry {
                title: context.title.clone().into(),
                kind: context.kind,
                pinned: context.pinned,
                unread: context.unread,
                needs_me,
                id,
            })
        })
        .collect()
}

impl Render for ContextStack {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let store = self.store.read(cx).store();
        let rows = entries(store, self.current.as_ref());
        let has_others = store.contexts().count() > rows.len();
        let theme = cx.theme();
        let (muted, warning, border, sidebar) =
            (theme.muted_foreground, theme.warning, theme.border, theme.sidebar);

        let list = rows.into_iter().map(|entry| {
            let selected = self.current.as_ref() == Some(&entry.id);
            let icon = match entry.kind {
                ContextKind::Direct => IconName::User,
                ContextKind::Group => IconName::Building2,
                ContextKind::Space => IconName::LayoutDashboard,
            };
            let id = entry.id.clone();
            Button::new(domain_id("stack", &entry.id))
                .ghost()
                .small()
                .w_full()
                .selected(selected)
                .on_click(cx.listener(move |this, _, _, cx| this.select(id.clone(), cx)))
                .child(
                    h_flex()
                        .w_full()
                        .min_w_0()
                        .gap_2()
                        .child(Icon::new(icon).small())
                        .child(div().flex_1().min_w_0().truncate().child(entry.title))
                        .when(entry.pinned, |row| {
                            row.child(Icon::new(IconName::Star).xsmall().text_color(muted))
                        })
                        .when(entry.needs_me > 0, |row| {
                            row.child(div().text_xs().text_color(warning).child(tr_args(
                                "stack.needs_me",
                                &[("count", &entry.needs_me.to_string())],
                            )))
                        })
                        .when(entry.needs_me == 0 && entry.unread > 0, |row| {
                            row.child(
                                div().text_xs().text_color(muted).child(entry.unread.to_string()),
                            )
                        }),
                )
        });

        let store_handle = self.store.downgrade();
        let this = cx.weak_entity();
        v_flex()
            .id("context-stack")
            .h_full()
            .w_56()
            .flex_shrink_0()
            .gap_1()
            .p_2()
            .bg(sidebar)
            .border_r_1()
            .border_color(border)
            .child(div().px_2().py_1().text_xs().text_color(muted).child(tr("stack.title")))
            .children(list)
            .child(div().flex_1())
            .when(has_others, |stack| {
                stack.child(
                    Button::new("stack-switch")
                        .ghost()
                        .small()
                        .w_full()
                        .icon(IconName::ChevronsUpDown)
                        .label(tr("stack.switch"))
                        .dropdown_menu(move |mut menu, _, cx| {
                            let Some(store) = store_handle.upgrade() else { return menu };
                            let contexts: Vec<(ContextId, String)> = store
                                .read(cx)
                                .store()
                                .contexts()
                                .map(|c| (c.id.clone(), c.title.clone()))
                                .collect();
                            for (id, title) in contexts {
                                let this = this.clone();
                                menu = menu.item(PopupMenuItem::new(title).on_click(
                                    move |_, _, cx| {
                                        let id = id.clone();
                                        this.update(cx, |stack, cx| stack.select(id, cx)).ok();
                                    },
                                ));
                            }
                            menu
                        }),
                )
            })
    }
}
