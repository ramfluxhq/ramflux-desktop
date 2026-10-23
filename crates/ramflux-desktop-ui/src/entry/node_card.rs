// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The node summary shown during entry (NODE-01 shape): node id, identity
//! key fingerprint, endpoints, pin source, health, and a trust-on-first-use
//! warning for nodes not pinned by the shipped bootstrap list.

use gpui_kit::component::alert::Alert;
use gpui_kit::component::description_list::DescriptionList;
use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, IntoElement, SharedString, Window};
use ramflux_desktop_core::{Node, PinSource};

use crate::i18n::{self, tr};
use crate::ids::domain_id;

/// One node card.
#[derive(IntoElement)]
pub struct NodeCard {
    node: Node,
}

impl NodeCard {
    /// A card for `node`.
    pub fn new(node: Node) -> Self {
        Self { node }
    }
}

impl RenderOnce for NodeCard {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let node = self.node;
        let fingerprint: SharedString =
            node.fingerprint.clone().map_or_else(|| tr("node.fingerprint_unknown"), Into::into);
        let endpoints: SharedString = if node.endpoints.is_empty() {
            tr("status.none")
        } else {
            node.endpoints.join("\n").into()
        };
        let first_contact = node.pin_source != PinSource::Bootstrap;
        v_flex()
            .id(domain_id("node", &node.id))
            .gap_3()
            .p_4()
            .border_1()
            .border_color(cx.theme().border)
            .rounded(cx.theme().radius_lg)
            .child(
                DescriptionList::new()
                    .columns(1)
                    .item(tr("node.id"), SharedString::from(node.id.to_string()), 1)
                    .item(tr("node.fingerprint"), fingerprint, 1)
                    .item(tr("node.endpoints"), endpoints, 1)
                    .item(tr("node.pin_source"), i18n::pin_source(node.pin_source), 1)
                    .item(tr("node.health"), i18n::node_health(node.health), 1),
            )
            .when(first_contact, |card| {
                card.child(
                    Alert::warning(domain_id("tofu", &node.id), tr("node.tofu_body"))
                        .title(tr("node.tofu_title")),
                )
            })
    }
}
