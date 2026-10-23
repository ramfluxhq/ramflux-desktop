// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Settings · Nodes: every node profile with its NODE-01 fields and last
//! probe result (gateway and relay), and the local actions: probe, make
//! default, enable or disable (the shipped node can be disabled once another
//! is usable), and remove (disabled with a reason while an account is bound).

use gpui_kit::component::StyledExt as _;
use gpui_kit::component::button::Button;
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

/// A node with whether an account is bound to it.
#[derive(Clone)]
struct Row {
    node: Node,
    bound: bool,
}

/// The nodes section.
pub fn page(context: &SectionContext, cx: &App) -> SettingPage {
    let store = context.store.read(cx).store();
    let rows: Vec<Row> = store
        .nodes()
        .map(|node| Row {
            bound: store.accounts().any(|a| a.node_id.as_ref() == Some(&node.id)),
            node: node.clone(),
        })
        .collect();
    let sink = context.sink(SettingsSection::Nodes);
    let list = SettingItem::render(move |_, _, cx: &mut App| {
        if rows.is_empty() {
            return note("settings.nodes.none", cx);
        }
        v_flex()
            .id("settings-nodes")
            .gap_3()
            .children(rows.iter().map(|row| node_row(row, &sink, cx)))
            .into_any_element()
    });
    SettingPage::new(tr("settings.section.nodes"))
        .description(tr("settings.nodes.description"))
        .group(SettingGroup::new().item(list))
}

fn probe_line(label: &str, outcome: &ProbeOutcome, cx: &App) -> AnyElement {
    let (status, color) = if outcome.ok {
        (
            outcome.latency_ms.map_or_else(
                || tr("settings.nodes.ok"),
                |ms| tr_args("settings.nodes.ok_ms", &[("ms", &ms.to_string())]),
            ),
            cx.theme().success,
        )
    } else {
        (tr("settings.nodes.failed"), cx.theme().danger)
    };
    h_flex()
        .gap_2()
        .text_xs()
        .child(div().w_16().text_color(cx.theme().muted_foreground).child(tr(label)))
        .child(div().text_color(color).child(status))
        .when(!outcome.detail.is_empty(), |row| {
            row.child(div().text_color(cx.theme().muted_foreground).child(outcome.detail.clone()))
        })
        .into_any_element()
}

fn action(
    sink: &EventSink,
    id: &'static str,
    node: &NodeId,
    action: NodeAction,
    label: SharedString,
) -> Button {
    let sink = sink.clone();
    let node_id = node.clone();
    Button::new(id).xsmall().outline().label(label).on_click(move |_, window, cx| {
        sink.emit(UiEventKind::NodeActionPressed { node_id: node_id.clone(), action }, window, cx);
    })
}

fn node_row(row: &Row, sink: &EventSink, cx: &App) -> AnyElement {
    let node = &row.node;
    let mut badges = vec![i18n::pin_source(node.pin_source)];
    if node.builtin {
        badges.push(tr("settings.nodes.builtin"));
    }
    if node.is_default {
        badges.push(tr("settings.nodes.default"));
    }
    if !node.enabled {
        badges.push(tr("settings.nodes.disabled"));
    }
    if !node.confirmed {
        badges.push(tr("node.pending"));
    }
    let probe =
        node.probe.as_ref().map(|probe| {
            v_flex()
                .gap_1()
                .child(div().text_xs().text_color(cx.theme().muted_foreground).child(tr_args(
                    "settings.nodes.probed",
                    &[("time", &i18n::date_time(probe.at))],
                )))
                .child(probe_line("settings.nodes.gateway", &probe.gateway, cx))
                .child(probe_line("settings.nodes.relay", &probe.relay, cx))
        });
    let remove =
        action(sink, "node-remove", &node.id, NodeAction::Remove, tr("settings.nodes.remove"))
            .disabled(row.bound)
            .when(row.bound, |b| b.tooltip(tr("settings.nodes.bound")));
    v_flex()
        .id(domain_id("settings-node", &node.id))
        .gap_2()
        .p_3()
        .border_1()
        .border_color(cx.theme().border)
        .rounded(cx.theme().radius_lg)
        .child(
            h_flex()
                .gap_2()
                .child(div().text_sm().font_semibold().child(node.id.to_string()))
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(badges.join(" · ")),
                )
                .child(div().flex_1())
                .child(div().text_xs().child(i18n::node_health(node.health))),
        )
        .child(
            div()
                .text_xs()
                .font_family(cx.theme().mono_font_family.clone())
                .text_color(cx.theme().muted_foreground)
                .child(node.fingerprint.clone().unwrap_or_default()),
        )
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(node.endpoints.join(", ")),
        )
        .children(probe)
        .when(node.probe.is_none(), |col| {
            col.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr("settings.nodes.never_probed")),
            )
        })
        .child(
            h_flex()
                .flex_wrap()
                .gap_2()
                .child(
                    action(
                        sink,
                        "node-probe",
                        &node.id,
                        NodeAction::Probe,
                        tr("settings.nodes.probe"),
                    )
                    .icon(IconName::RefreshCw),
                )
                .when(!node.is_default && node.enabled && node.confirmed, |row| {
                    row.child(action(
                        sink,
                        "node-default",
                        &node.id,
                        NodeAction::SetDefault,
                        tr("settings.nodes.make_default"),
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
                    ))
                })
                .child(remove),
        )
        .when(row.bound, |col| {
            col.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr("settings.nodes.bound")),
            )
        })
        .into_any_element()
}
