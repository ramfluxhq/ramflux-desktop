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
use gpui_kit::component::{ActiveTheme as _, IconName, Sizable as _, h_flex, tooltip::Tooltip};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, Entity, Hsla, IntoElement, Render, SharedString, Subscription, Window, div,
};
use ramflux_desktop_core::{LockState, NodeHealth, SchemaGate, SourceStatus, Store};

use crate::app::{StoreEvent, StoreHandle};
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
            StoreEvent::Changed(changes)
                if changes.touches_status() || changes.is_clock_moved() =>
            {
                cx.notify()
            }
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
    /// Needs the user's attention (a problem), not just a fact.
    attention: bool,
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
    let node_attention =
        node.is_some_and(|n| matches!(n.health, NodeHealth::Degraded | NodeHealth::Unreachable));
    let source = handle.shown_source_status();
    let source_color = match source {
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
        attention: true,
    });
    let mut fields = vec![
        Field {
            id: "status-device",
            label: tr("status.label.device"),
            value: device,
            color: None,
            attention: false,
        },
        Field {
            id: "status-node",
            label: tr("status.label.node"),
            value: node_value,
            color: node_color,
            attention: node_attention,
        },
        Field {
            id: "status-projection",
            label: tr("status.label.projection"),
            value: projection,
            color: (rejected > 0).then_some(theme.warning),
            attention: rejected > 0,
        },
        Field {
            id: "status-source",
            label: tr("status.label.source"),
            value: i18n::source_status(source),
            color: source_color,
            attention: source != SourceStatus::Connected,
        },
    ];
    fields.extend(schema_line(handle, cx));
    fields.extend(failure);
    fields
}

/// The account-status schema line: the generation the daemon reports, or
/// why the gate is closed.
fn schema_line(handle: &StoreHandle, cx: &App) -> Option<Field> {
    let (account, gate) = super::schema_gate::active(handle)?;
    let generation = handle.store().schema_generation(&account);
    if generation.is_none() && gate.is_open() {
        return None;
    }
    let value = match (generation, gate) {
        (Some(n), _) => tr_args("status.schema.generation", &[("generation", &n.to_string())]),
        (None, SchemaGate::Unresolved) => tr("status.schema.checking"),
        (None, SchemaGate::Failed | SchemaGate::ResetRequired) => tr("status.schema.unavailable"),
        (None, _) => tr("status.schema.unknown"),
    };
    Some(Field {
        id: "status-schema",
        label: tr("status.label.schema"),
        value,
        color: (!gate.is_open()).then(|| cx.theme().warning),
        attention: !gate.is_open(),
    })
}

impl Render for StatusStrip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Design B38 / 10: nothing while all is well; otherwise one quiet
        // strip at the top of the content area naming what needs attention
        // (networking, a node or the daemon not fine, a rejected projection,
        // the latest failure, other accounts' attention, decisions waiting).
        let handle = self.store.read(cx);
        let fields = fields(handle, cx);
        let account = handle
            .active_account()
            .and_then(|id| handle.store().account(id))
            .or_else(|| handle.store().accounts().find(|a| a.lock_state == LockState::Unlocked));
        let (network_title, network_detail) = if self.show_commands && account.is_none() {
            (tr("network_header.connecting"), None)
        } else {
            super::networking_header::header(
                account.filter(|_| self.show_commands),
                account.and_then(|a| a.networking_failure.as_ref()),
                account.and_then(|a| handle.session()?.networking_retry_remaining(&a.id)),
            )
        };
        let healthy_title = network_title == tr("app.name");
        let palette = ramflux_desktop_navigation::palette(cx);
        let muted = palette.secondary;
        let foreground = palette.text;
        let chips: Vec<_> = fields
            .into_iter()
            .filter(|f| f.attention)
            .map(|field| {
                // A failure appears while the user is elsewhere: a status region
                // lets a screen reader announce it without moving focus.
                let full = field.value.clone();
                let is_failure = field.id == "status-failure";
                h_flex()
                    .id(field.id)
                    .when(is_failure, |chip| chip.role(gpui_kit::Role::Status).flex_1())
                    .when(!is_failure, |chip| chip.flex_none())
                    .test_support()
                    .gap_1()
                    .min_w_0()
                    .overflow_hidden()
                    .child(
                        div().flex_none().whitespace_nowrap().text_color(muted).child(field.label),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("{}-value", field.id)))
                            .aria_label(field.value.clone())
                            .test_support()
                            .min_w_0()
                            .when(is_failure, |value| value.flex_1())
                            .when(!is_failure, |value| value.max_w(gpui_kit::rems(24.)))
                            .truncate()
                            .text_color(field.color.unwrap_or(foreground))
                            .tooltip(move |window, cx| Tooltip::new(full.clone()).build(window, cx))
                            .child(field.value),
                    )
                    .into_any_element()
            })
            .collect();
        // Design B38: only a connection or failure state is a banner; other
        // accounts' attention is in the identity popover, what needs me in
        // Tasks.
        let shown = !healthy_title || !chips.is_empty();
        // The connection state stays a status region even while it is fine,
        // so assistive technology can always read it.
        let title = div()
            .id("workspace-networking-title")
            .role(gpui_kit::Role::Status)
            .aria_label(network_title.clone())
            .test_support()
            .flex_none()
            .when(healthy_title, |title| title.size_0().overflow_hidden())
            .when(!healthy_title, |title| title.font_medium().child(network_title));
        h_flex()
            .id("status-strip")
            .test_support()
            .w_full()
            .min_w_0()
            .gap_3()
            .overflow_hidden()
            .when(shown, |strip| {
                strip
                    .min_h(gpui_kit::rems(34.0 / 15.0))
                    .px_4()
                    .bg(palette.surface)
                    .text_size(gpui_kit::rems(palette.size_secondary / 15.0))
                    .child(gpui_kit::component::Icon::new(IconName::Info).small().text_color(muted))
            })
            .child(title)
            .children(network_detail.map(|detail| {
                div()
                    .id("workspace-networking-detail")
                    .aria_label(detail.clone())
                    .test_support()
                    .min_w_0()
                    .truncate()
                    .text_color(muted)
                    .child(detail)
            }))
            .children(chips)
    }
}
