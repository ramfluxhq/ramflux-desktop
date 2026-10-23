// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The node card (NODE-01 summary): node id, identity key fingerprint,
//! endpoints, capabilities, record expiry, pin source and health. A node
//! trusted on first use carries a sticky warning. A discovered node offers
//! Confirm (audited; a trust-on-first-use pin needs a hold) and Not now.

use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::description_list::DescriptionList;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, IntoElement, SharedString, Window};
use ramflux_desktop_core::{Node, NodeAction, UiEventKind};

use crate::bubble::EventSink;
use crate::i18n::{self, tr};
use crate::ids::domain_id;

/// One node card.
#[derive(IntoElement)]
pub struct NodeCard {
    node: Node,
    sink: Option<EventSink>,
}

impl NodeCard {
    /// A read-only card for `node`.
    pub fn new(node: Node) -> Self {
        Self { node, sink: None }
    }

    /// Offers Confirm and Not now for a discovered node, emitting through `sink`.
    pub fn actions(mut self, sink: EventSink) -> Self {
        self.sink = Some(sink);
        self
    }
}

impl RenderOnce for NodeCard {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let node = self.node;
        let fingerprint: SharedString =
            node.fingerprint.clone().map_or_else(|| tr("node.fingerprint_unknown"), Into::into);
        let join = |items: &[String]| -> SharedString {
            if items.is_empty() { tr("status.none") } else { items.join("\n").into() }
        };
        let expiry: SharedString =
            node.expires_at.map_or_else(|| tr("grant.no_expiry"), |at| i18n::date_time(at).into());
        let tofu = node.pin_source.is_trust_on_first_use();
        let pending = !node.confirmed;
        let actions = self.sink.filter(|_| pending).map(|sink| {
            let confirm = sink.clone();
            let node_id = node.id.clone();
            let decline_id = node.id.clone();
            h_flex()
                .gap_2()
                .child(
                    Button::new("node-confirm")
                        .primary()
                        .small()
                        .label(tr(if tofu { "node.confirm_hold" } else { "node.confirm" }))
                        .on_click(move |_, window, cx| {
                            confirm.emit(
                                UiEventKind::ConfirmNodePressed { node_id: node_id.clone() },
                                window,
                                cx,
                            );
                        }),
                )
                .child(
                    Button::new("node-decline")
                        .outline()
                        .small()
                        .label(tr("node.decline"))
                        .on_click(move |_, window, cx| {
                            sink.emit(
                                UiEventKind::NodeActionPressed {
                                    node_id: decline_id.clone(),
                                    action: NodeAction::Remove,
                                },
                                window,
                                cx,
                            );
                        }),
                )
        });
        v_flex()
            .id(domain_id("node", &node.id))
            .gap_3()
            .p_4()
            .border_1()
            .border_color(if pending && tofu { cx.theme().warning } else { cx.theme().border })
            .rounded(cx.theme().radius_lg)
            .child(
                DescriptionList::new()
                    .columns(1)
                    .item(tr("node.id"), SharedString::from(node.id.to_string()), 1)
                    .item(tr("node.fingerprint"), fingerprint, 1)
                    .item(tr("node.endpoints"), join(&node.endpoints), 1)
                    .item(
                        tr("node.capabilities"),
                        SharedString::from(node.capabilities.join(", ")),
                        1,
                    )
                    .item(tr("node.expires"), expiry, 1)
                    .item(tr("node.pin_source"), i18n::pin_source(node.pin_source), 1)
                    .item(tr("node.health"), i18n::node_health(node.health), 1)
                    .item(
                        tr("node.state"),
                        tr(if pending { "node.pending" } else { "node.confirmed" }),
                        1,
                    ),
            )
            .when(tofu, |card| {
                card.child(
                    Alert::warning(domain_id("tofu", &node.id), tr("node.tofu_body"))
                        .title(tr("node.tofu_title")),
                )
            })
            .children(actions)
    }
}
