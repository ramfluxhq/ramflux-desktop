// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The node step: discovered nodes waiting for a decision as full NODE-01
//! cards, the add-node form, then the nodes already confirmed as compact
//! rows. The hold strip for trust-on-first-use pins is the page's footer.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{Context, IntoElement, Window, div};
use ramflux_desktop_core::{Node, UiEventKind};

use super::{EntryPage, NodeCard};
use crate::i18n::tr;

impl EntryPage {
    /// Submits the typed node address for discovery and clears the input.
    pub fn add_node(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let url = self.inputs.node_url.read(cx).value().trim().to_owned();
        if url.is_empty() {
            return;
        }
        self.inputs.node_url.update(cx, |input, cx| input.set_value("", window, cx));
        self.sink.emit(UiEventKind::AddNodeRequested { url }, window, cx);
    }

    pub(super) fn render_node_step(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let store = self.store.read(cx).store();
        let (pending_nodes, known): (Vec<Node>, Vec<Node>) =
            store.nodes().cloned().partition(|n| !n.confirmed);
        let waiting = pending_nodes.is_empty() && known.is_empty();
        let pending = !pending_nodes.is_empty();
        let usable = known.iter().any(|n| n.enabled);
        let cards =
            pending_nodes.into_iter().map(|node| NodeCard::new(node).actions(self.sink.clone()));
        let muted = cx.theme().muted_foreground;
        let rows = known.into_iter().map(move |node| {
            h_flex()
                .id(crate::ids::domain_id("node-row", &node.id))
                .gap_2()
                .text_sm()
                .child(div().child(node.id.to_string()))
                .child(div().text_xs().text_color(muted).child(format!(
                    "{} · {}",
                    crate::i18n::pin_source(node.pin_source),
                    crate::i18n::node_health(node.health)
                )))
        });
        v_flex()
            .id("entry-node")
            .gap_3()
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr("entry.node.intro")),
            )
            .when(waiting, |step| {
                step.child(
                    h_flex()
                        .gap_3()
                        .child(Spinner::new())
                        .child(div().text_sm().child(tr("entry.node.waiting"))),
                )
            })
            .children(cards)
            .child(
                v_flex()
                    .gap_2()
                    .pt_2()
                    .child(div().text_sm().child(tr("entry.node.add_title")))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Input::new(&self.inputs.node_url)
                                    .id("node-url")
                                    .aria_label(tr("entry.node.add_title"))
                                    .flex_1(),
                            )
                            .child(
                                Button::new("node-add")
                                    .outline()
                                    .icon(IconName::Search)
                                    .label(tr("entry.node.add"))
                                    .on_click(
                                        cx.listener(|this, _, window, cx| {
                                            this.add_node(window, cx)
                                        }),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(tr("entry.node.add_help")),
                    ),
            )
            .when(usable, |step| {
                step.child(div().pt_2().text_sm().child(tr("entry.node.known"))).children(rows)
            })
            .when(usable && !pending && self.review_nodes, |step| {
                step.child(
                    Button::new("entry-continue")
                        .primary()
                        .small()
                        .label(tr("entry.node.continue"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.review_nodes = false;
                            cx.notify();
                        })),
                )
            })
    }
}
