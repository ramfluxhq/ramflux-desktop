// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Settings · Nodes: every node profile with its NODE-01 fields and last
//! probe result (gateway and relay), and the local actions: probe, make
//! default, enable or disable (the shipped node can be disabled once another
//! is usable), and remove (disabled with a reason while an account is bound).

use gpui_kit::component::StyledExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::setting::{SettingGroup, SettingItem, SettingPage};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Sizable as _, h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, SharedString, div};
use ramflux_desktop_core::{Node, NodeAction, NodeId, ProbeOutcome, UiEventKind};

use super::{SectionContext, SettingsSection, note};
use crate::bubble::EventSink;
use crate::i18n::{self, tr, tr_args};
use crate::ids::domain_id;
use crate::style::DesignButton as _;

/// A node with whether an account is bound to it.
#[derive(Clone)]
struct Row {
    node: Node,
    bound: bool,
}

/// The nodes section (design G09): an ⓘ with what nodes are, the default
/// node as its own section (chips, fingerprint field, endpoint / gateway /
/// relay rows, actions, and why it cannot be removed), then the others
/// under "Backup nodes".
pub fn page(context: &SectionContext, cx: &App) -> SettingPage {
    let store = context.store.read(cx).store();
    let mut rows: Vec<Row> = store
        .nodes()
        .map(|node| Row {
            bound: store.accounts().any(|a| a.node_id.as_ref() == Some(&node.id)),
            node: node.clone(),
        })
        .collect();
    rows.sort_by_key(|row| !row.node.is_default);
    let sink = context.sink(SettingsSection::Nodes);
    let list = SettingItem::render(move |_, _, cx: &mut App| {
        if rows.is_empty() {
            return note("settings.nodes.none", cx);
        }
        let palette = ramflux_desktop_navigation::palette(cx);
        let (default, others): (Vec<&Row>, Vec<&Row>) =
            rows.iter().partition(|row| row.node.is_default);
        v_flex()
            .id("settings-nodes")
            .gap_6()
            .child(
                div()
                    .id("settings-nodes-info")
                    .size(gpui_kit::rems(24.0 / 15.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(palette.secondary)
                    .tooltip(|window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(tr("settings.nodes.description"))
                            .build(window, cx)
                    })
                    .child(gpui_kit::component::Icon::new(IconName::Info).small()),
            )
            .children(default.into_iter().map(|row| node_section(row, &sink, None, cx)))
            .when(!others.is_empty(), |list| list.child(node_group(&others, &sink, cx)))
            .into_any_element()
    });
    SettingPage::new(tr("settings.section.nodes")).group(SettingGroup::new().item(list))
}

/// The backup nodes: one section, a row and its actions per node.
fn node_group(rows: &[&Row], sink: &EventSink, cx: &App) -> AnyElement {
    let mut section = v_flex().gap_3();
    for row in rows {
        section = section.child(node_section(row, sink, Some(()), cx));
    }
    super::design::section("settings-nodes-backup", tr("settings.nodes.backup_title"), cx)
        .child(section)
        .into_any_element()
}

fn probe_detail(outcome: Option<&ProbeOutcome>) -> (SharedString, Option<SharedString>) {
    match outcome {
        None => (tr("settings.nodes.never_probed"), None),
        Some(outcome) if outcome.ok => (
            outcome.latency_ms.map_or_else(
                || tr("settings.nodes.ok"),
                |ms| tr_args("settings.nodes.ok_ms", &[("ms", &ms.to_string())]),
            ),
            Some(tr("settings.nodes.ok")),
        ),
        Some(outcome) => (
            if outcome.detail.is_empty() {
                tr("settings.nodes.failed")
            } else {
                i18n::join(&tr("settings.nodes.failed"), &outcome.detail)
            },
            Some(tr("settings.nodes.failed")),
        ),
    }
}

fn action(
    sink: &EventSink,
    id: &'static str,
    node: &NodeId,
    action: NodeAction,
    label: SharedString,
    cx: &App,
) -> Button {
    let sink = sink.clone();
    let node_id = node.clone();
    Button::new(id).design_secondary(cx).label(label).on_click(move |_, window, cx| {
        sink.emit(UiEventKind::NodeActionPressed { node_id: node_id.clone(), action }, window, cx);
    })
}

/// One node. The default node is a whole section (`compact` is `None`);
/// a backup node is a row with its chips and actions.
fn node_section(row: &Row, sink: &EventSink, compact: Option<()>, cx: &App) -> AnyElement {
    let node = &row.node;
    let palette = ramflux_desktop_navigation::palette(cx);
    let mut chips = Vec::new();
    if node.is_default {
        chips.push(tr("settings.nodes.default"));
    }
    if node.builtin {
        chips.push(tr("settings.nodes.builtin"));
    }
    if !node.enabled {
        chips.push(tr("settings.nodes.disabled"));
    }
    if !node.confirmed {
        chips.push(tr("node.pending"));
    }
    chips.push(i18n::pin_source(node.pin_source));
    let chip_row = h_flex().gap_2().children(chips.into_iter().map(|c| super::design::chip(c, cx)));
    let remove = Button::new("node-remove")
        .ghost()
        .h(gpui_kit::rems(32.0 / 15.0))
        .px(gpui_kit::rems(12.0 / 15.0))
        .bg(palette.danger_bg)
        .text_color(palette.danger)
        .label(tr("settings.nodes.remove"))
        .disabled(row.bound)
        .when(row.bound, |b| b.tooltip(tr("settings.nodes.bound")))
        .on_click({
            let sink = sink.clone();
            let node_id = node.id.clone();
            move |_, window, cx| {
                sink.emit(
                    UiEventKind::NodeActionPressed {
                        node_id: node_id.clone(),
                        action: NodeAction::Remove,
                    },
                    window,
                    cx,
                );
            }
        });
    let actions = h_flex()
        .flex_wrap()
        .gap_2()
        .child(action(
            sink,
            "node-probe",
            &node.id,
            NodeAction::Probe,
            tr("settings.nodes.probe"),
            cx,
        ))
        .when(!node.is_default && node.enabled && node.confirmed, |row| {
            row.child(action(
                sink,
                "node-default",
                &node.id,
                NodeAction::SetDefault,
                tr("settings.nodes.make_default"),
                cx,
            ))
        })
        .when(!node.is_default, |row| {
            let (key, enabled) = if node.enabled {
                ("settings.nodes.disable", false)
            } else {
                ("settings.nodes.enable", true)
            };
            row.child(action(
                sink,
                "node-enabled",
                &node.id,
                NodeAction::SetEnabled(enabled),
                tr(key),
                cx,
            ))
        })
        .child(remove);
    let probe = node.probe.as_ref();
    let (gateway, gateway_chip) = probe_detail(probe.map(|p| &p.gateway));
    let (relay, relay_chip) = probe_detail(probe.map(|p| &p.relay));
    let probed =
        probe.map(|p| tr_args("settings.nodes.probed", &[("time", &i18n::date_time(p.at))]));
    let row_with = |title: SharedString, detail: SharedString, chip: Option<SharedString>| {
        super::design::row(title, detail, chip.map(|c| super::design::chip(c, cx)), cx)
    };
    let body = v_flex()
        .id(domain_id("settings-node", &node.id))
        .gap_3()
        .w_full()
        .child(chip_row)
        .when(compact.is_none(), |col| {
            col.child(super::presentation::field(
                tr("settings.nodes.fingerprint"),
                div()
                    .w_full()
                    .px_3()
                    .py_2()
                    .rounded(gpui_kit::rems(palette.radius_input / 15.0))
                    .border_1()
                    .border_color(palette.line_strong)
                    .bg(palette.surface_white)
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_size(gpui_kit::rems(palette.size_body / 15.0))
                    .child(node.fingerprint.clone().unwrap_or_default()),
                cx,
            ))
        })
        .child(row_with(tr("settings.nodes.endpoints"), crate::i18n::list(&node.endpoints), None))
        .child(row_with(
            tr("settings.nodes.gateway"),
            match &probed {
                Some(p) => i18n::join(&gateway, p),
                None => gateway,
            },
            gateway_chip,
        ))
        .child(row_with(tr("settings.nodes.relay"), relay, relay_chip))
        .child(row_with(tr("settings.nodes.health"), i18n::node_health(node.health), None))
        .child(actions)
        .when(row.bound, |col| {
            col.child(
                v_flex()
                    .id("settings-node-bound")
                    .gap_1()
                    .p_4()
                    .rounded(gpui_kit::rems(palette.radius_md / 15.0))
                    .bg(palette.surface)
                    .child(
                        div()
                            .text_size(gpui_kit::rems(palette.size_body / 15.0))
                            .font_medium()
                            .child(tr("settings.nodes.bound_title")),
                    )
                    .child(super::design::help(tr("settings.nodes.bound"), cx)),
            )
        });
    match compact {
        Some(()) => v_flex()
            .gap_2()
            .child(
                div()
                    .text_size(gpui_kit::rems(palette.size_body / 15.0))
                    .font_medium()
                    .child(node.id.to_string()),
            )
            .child(body)
            .into_any_element(),
        None => super::design::section(
            domain_id("settings-node-section", &node.id),
            SharedString::from(node.id.to_string()),
            cx,
        )
        .child(body)
        .into_any_element(),
    }
}
