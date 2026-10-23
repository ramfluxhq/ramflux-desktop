// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The terminal status strip in the window title bar: the account switcher
//! (with background accounts' attention), device, node, projection and
//! source state, plus the palette and "needs me" commands.
//!
//! The strip re-renders only when accounts, nodes, the source status or
//! attention change.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::StyledExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Sizable as _, TitleBar, h_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, Entity, Hsla, IntoElement, Render, SharedString, Subscription, Window, div,
};
use ramflux_desktop_core::{AccountId, LockState, NodeHealth, SourceStatus, Store};

use crate::actions::{GoToNeedsMe, OpenPalette, OpenSettings, ToggleDevPanel, ToggleTheme};
use crate::app::{LOCAL_ACCOUNT, StoreEvent, StoreHandle};
use crate::i18n::{self, tr, tr_args};

/// The status strip view.
pub struct StatusStrip {
    store: Entity<StoreHandle>,
    show_commands: bool,
    _subscription: Subscription,
}

impl StatusStrip {
    /// A strip reading `store`.
    pub fn new(store: Entity<StoreHandle>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.subscribe(&store, |_, _, event: &StoreEvent, cx| match event {
            StoreEvent::Changed(changes) if changes.touches_status() => cx.notify(),
            StoreEvent::BackgroundChanged(_) => cx.notify(),
            _ => {}
        });
        Self { store, show_commands: false, _subscription: subscription }
    }

    /// Shows the workspace commands (needs me) only on the workspace page.
    pub fn set_show_commands(&mut self, show: bool, cx: &mut Context<Self>) {
        if self.show_commands != show {
            self.show_commands = show;
            cx.notify();
        }
    }
}

struct Field {
    id: &'static str,
    label: SharedString,
    value: SharedString,
    color: Option<Hsla>,
}

fn fields(handle: &StoreHandle, cx: &App) -> Vec<Field> {
    let store: &Store = handle.store();
    let theme = cx.theme();
    let account = store
        .accounts()
        .find(|a| a.lock_state == LockState::Unlocked)
        .or_else(|| store.accounts().next());
    let device: SharedString = account
        .and_then(|a| a.device_id.clone())
        .map_or_else(|| tr("status.none"), SharedString::from);
    let node = account
        .and_then(|a| a.node_id.as_ref())
        .and_then(|id| store.node(id))
        .or_else(|| store.nodes().next());
    let (node_value, node_color) = match node {
        Some(node) => {
            let color = match node.health {
                NodeHealth::Healthy => None,
                NodeHealth::Unknown => Some(theme.muted_foreground),
                NodeHealth::Degraded => Some(theme.warning),
                NodeHealth::Unreachable => Some(theme.danger),
            };
            (
                tr_args(
                    "status.node.value",
                    &[("node", node.id.as_str()), ("health", &i18n::node_health(node.health))],
                ),
                color,
            )
        }
        None => (tr("status.none"), None),
    };
    let source_color = match store.source_status() {
        SourceStatus::Connected => None,
        SourceStatus::Locked => Some(theme.warning),
        SourceStatus::Disconnected => Some(theme.danger),
    };
    let rejected = store.rejections().len();
    let projection = if rejected == 0 {
        tr_args("status.projection.value", &[("seq", &store.last_seq().to_string())])
    } else {
        tr_args(
            "status.projection.rejected",
            &[("seq", &store.last_seq().to_string()), ("count", &rejected.to_string())],
        )
    };
    let failure = store.failures().last().map(|f| Field {
        id: "status-failure",
        label: tr("backend.failure.label"),
        value: i18n::failure(f),
        color: Some(theme.danger),
    });
    let mut fields = vec![
        Field { id: "status-device", label: tr("status.label.device"), value: device, color: None },
        Field {
            id: "status-node",
            label: tr("status.label.node"),
            value: node_value,
            color: node_color,
        },
        Field {
            id: "status-projection",
            label: tr("status.label.projection"),
            value: projection,
            color: (rejected > 0).then_some(theme.warning),
        },
        Field {
            id: "status-source",
            label: tr("status.label.source"),
            value: i18n::source_status(store.source_status()),
            color: source_color,
        },
    ];
    fields.extend(failure);
    fields
}

/// The account switcher: the rendered account, the "needs me" count of the
/// other accounts, and a menu that switches stores (nothing reloads).
fn account_switcher(
    handle: &StoreHandle,
    store: gpui_kit::WeakEntity<StoreHandle>,
    cx: &App,
) -> impl IntoElement {
    let active = handle.active_account().cloned();
    let accounts: Vec<(AccountId, String, usize)> = handle
        .accounts()
        .map(|id| {
            let name = handle.account_label(id).unwrap_or_else(|| id.to_string());
            (id.clone(), name, handle.needs_me_count(id))
        })
        .collect();
    let label: SharedString = match &active {
        Some(id) if id.as_str() != LOCAL_ACCOUNT => {
            handle.account_label(id).map_or_else(|| tr("status.none"), SharedString::from)
        }
        _ => tr("status.account.local"),
    };
    let background: usize =
        accounts.iter().filter(|(id, _, _)| Some(id) != active.as_ref()).map(|(_, _, n)| n).sum();
    let warning = cx.theme().warning;
    h_flex()
        .id("status-account")
        .gap_1()
        .text_xs()
        .child(div().text_color(cx.theme().muted_foreground).child(tr("status.label.account")))
        .child(
            Button::new("account-switcher")
                .ghost()
                .xsmall()
                .label(label)
                .when(accounts.len() > 1, |b| b.icon(IconName::ChevronsUpDown))
                .dropdown_menu(move |mut menu, _, _| {
                    for (id, name, count) in &accounts {
                        let store = store.clone();
                        let target = id.clone();
                        let text = if *count > 0 {
                            tr_args(
                                "status.account.with_count",
                                &[("name", name), ("count", &count.to_string())],
                            )
                        } else {
                            SharedString::from(name.clone())
                        };
                        menu = menu.item(
                            PopupMenuItem::new(text).checked(active.as_ref() == Some(id)).on_click(
                                move |_, _, cx| {
                                    store
                                        .update(cx, |store, cx| {
                                            store.set_active_account(&target, cx)
                                        })
                                        .ok();
                                },
                            ),
                        );
                    }
                    menu
                }),
        )
        .when(background > 0, |row| {
            row.child(
                div()
                    .id("status-background-attention")
                    .text_color(warning)
                    .child(tr_args(
                        "status.account.background",
                        &[("count", &background.to_string())],
                    ))
                    .test_support(),
            )
        })
}

impl Render for StatusStrip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let handle = self.store.read(cx);
        let fields = fields(handle, cx);
        let switcher = account_switcher(handle, self.store.downgrade(), cx);
        let needs_me = handle.attention_across_accounts().len();
        let muted = cx.theme().muted_foreground;
        let foreground = cx.theme().foreground;
        let chips = fields.into_iter().map(|field| {
            h_flex()
                .id(field.id)
                .gap_1()
                .text_xs()
                .child(div().text_color(muted).child(field.label))
                .child(
                    div()
                        .max_w(gpui_kit::rems(24.))
                        .truncate()
                        .text_color(field.color.unwrap_or(foreground))
                        .child(field.value),
                )
        });
        TitleBar::new()
            .child(
                h_flex()
                    .id("status-strip")
                    .gap_4()
                    .min_w_0()
                    .child(div().text_sm().font_semibold().child(tr("app.name")))
                    .child(switcher)
                    .children(chips),
            )
            .child(
                h_flex()
                    .gap_1()
                    .pr_2()
                    .when(self.show_commands, |row| {
                        row.child(
                            Button::new("open-palette")
                                .ghost()
                                .small()
                                .icon(IconName::Search)
                                .accessibility_label(tr("status.palette"))
                                .tooltip_with_action(tr("status.palette"), &OpenPalette, None)
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(OpenPalette), cx)
                                }),
                        )
                        .child(
                            Button::new("needs-me")
                                .small()
                                .when(needs_me > 0, |b| b.warning())
                                .when(needs_me == 0, |b| b.ghost())
                                .icon(IconName::Bell)
                                .label(tr_args(
                                    "status.needs_me",
                                    &[("count", &needs_me.to_string())],
                                ))
                                .tooltip_with_action(tr("status.needs_me_hint"), &GoToNeedsMe, None)
                                .disabled(needs_me == 0)
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(GoToNeedsMe), cx)
                                }),
                        )
                    })
                    .child(
                        Button::new("toggle-theme")
                            .ghost()
                            .small()
                            .icon(if cx.theme().is_dark() { IconName::Sun } else { IconName::Moon })
                            .accessibility_label(tr("status.toggle_theme"))
                            .tooltip_with_action(tr("status.toggle_theme"), &ToggleTheme, None)
                            .on_click(|_, window, cx| {
                                window.dispatch_action(Box::new(ToggleTheme), cx)
                            }),
                    )
                    .child(
                        Button::new("open-settings")
                            .ghost()
                            .small()
                            .icon(IconName::Settings)
                            .accessibility_label(tr("status.settings"))
                            .tooltip_with_action(tr("status.settings"), &OpenSettings, None)
                            .on_click(|_, window, cx| {
                                window.dispatch_action(Box::new(OpenSettings), cx)
                            }),
                    )
                    .when(cfg!(debug_assertions), |row| {
                        row.child(
                            Button::new("toggle-dev")
                                .ghost()
                                .small()
                                .icon(IconName::SquareTerminal)
                                .accessibility_label(tr("status.dev_panel"))
                                .tooltip_with_action(tr("status.dev_panel"), &ToggleDevPanel, None)
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(ToggleDevPanel), cx)
                                }),
                        )
                    }),
            )
    }
}
