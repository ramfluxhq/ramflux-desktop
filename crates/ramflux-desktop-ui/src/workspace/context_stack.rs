// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The compact context stack: pinned contexts, then contexts that need the
//! user (selector `context_stack`, at most six). It is a spatial switcher,
//! not a list of every conversation; every other context is reached only
//! through the command palette, which the footer opens. Choosing a context is
//! navigation (view-local), so it is reported to the workspace as a
//! [`ContextStackEvent`], not a UI event. The "+" in the header lists the
//! accepted contacts to start a conversation with (and adding a contact).

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Selectable as _, Sizable as _, h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{
    Context, Entity, EventEmitter, IntoElement, Render, SharedString, Subscription, Window, div,
};
use ramflux_desktop_core::{
    ContactLink, ContextId, ContextKind, Listing, ParticipantId, Store, selectors,
};

use crate::actions::OpenPalette;
use crate::app::{StoreEvent, StoreHandle};
use crate::i18n::{tr, tr_args};
use crate::ids::domain_id;

/// Reported to the workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextStackEvent {
    /// The user chose a context.
    Selected(ContextId),
    /// The user chose a contact to start a conversation with.
    NewConversation(ParticipantId),
    /// The user chose to add a contact first.
    AddContact,
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

    /// The contexts shown, top to bottom (what secondary-1..6 select).
    pub fn entry_ids(&self, store: &Store) -> Vec<ContextId> {
        entries(store, self.current.as_ref()).into_iter().map(|entry| entry.id).collect()
    }
}

/// The accessible name of an entry: its title and every state the row shows.
fn accessible_name(entry: &Entry) -> SharedString {
    let mut parts = vec![entry.title.to_string()];
    if entry.pinned {
        parts.push(tr("stack.pinned").to_string());
    }
    if entry.muted {
        parts.push(tr("stack.muted").to_string());
    }
    if entry.needs_me > 0 {
        parts
            .push(tr_args("stack.needs_me", &[("count", &entry.needs_me.to_string())]).to_string());
    } else if entry.unread > 0 {
        parts.push(tr_args("stack.unread", &[("count", &entry.unread.to_string())]).to_string());
    }
    parts.join(", ").into()
}

struct Entry {
    id: ContextId,
    title: SharedString,
    kind: ContextKind,
    pinned: bool,
    muted: bool,
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
            let needs_me = selectors::needs_me_count(store, &id);
            Some(Entry {
                title: context.title.clone().into(),
                kind: context.kind,
                pinned: context.pinned,
                muted: context.muted,
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
        let total = store.contexts().count();
        let others = total.saturating_sub(rows.len());
        // Nothing stacked: tell "still loading" from "no conversations".
        let placeholder = rows.is_empty().then(|| {
            let state = crate::list_state::of(store, Listing::Contexts, total == 0);
            crate::list_state::placeholder("stack-list-state", state, "stack.empty", cx)
        });
        let has_others = others > 0;
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
                .accessibility_label(accessible_name(&entry))
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
                        .when(entry.muted, |row| {
                            row.child(div().text_xs().text_color(muted).child(tr("stack.muted")))
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
                menu.separator().item(
                    PopupMenuItem::new(tr("stack.add_contact"))
                        .on_click(emit(ContextStackEvent::AddContact)),
                )
            });
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
            .child(
                h_flex()
                    .pl_2()
                    .child(
                        div().flex_1().py_1().text_xs().text_color(muted).child(tr("stack.title")),
                    )
                    .child(new_menu),
            )
            .children(list)
            .children(placeholder.flatten())
            .child(div().flex_1())
            .child(
                Button::new("stack-search")
                    .ghost()
                    .small()
                    .w_full()
                    .icon(IconName::Search)
                    .label(if has_others {
                        tr_args("stack.search_more", &[("count", &others.to_string())])
                    } else {
                        tr("stack.search")
                    })
                    .on_click(|_, window, cx| window.dispatch_action(Box::new(OpenPalette), cx)),
            )
    }
}
