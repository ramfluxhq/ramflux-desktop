// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The node card (NODE-01 summary), as the page of one decision: the node's
//! name and its one address, a neutral status line, and Cancel then "Trust
//! this node". The key fingerprint, the endpoints, the capabilities, the
//! record expiry, the pin source, the health and the state are one click
//! away in a collapsed "Technical details" disclosure; nothing asks the user
//! to compare a fingerprint with anyone. Pressing "Trust this node" is the
//! confirmation of a first-use pin: no second confirmation follows it.

use std::rc::Rc;

use crate::style::DesignButton as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::description_list::DescriptionList;
use gpui_kit::component::{ActiveTheme as _, IconName, Sizable as _, StyledExt as _, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, IntoElement, SharedString, Window, div};
use ramflux_desktop_core::{Node, NodeAction, PinSource, UiEventKind};
use ramflux_desktop_navigation::direction::{DirectionalExt as _, forward_icon, h_flex};
use ramflux_desktop_navigation::scale::{space, text_caption, text_secondary, text_subheading};

use crate::bubble::EventSink;
use crate::i18n::{self, tr};
use crate::ids::domain_id;

/// Flips a disclosure.
pub type Toggle = Rc<dyn Fn(&mut Window, &mut App)>;

/// The disclosure state a card shows: view-local, owned by the page.
pub struct Disclosure {
    /// Whether the technical details are showing.
    pub open: bool,
    /// Flips `open`.
    pub toggle: Toggle,
    /// The local service's socket and open accounts line, when known.
    pub socket: Option<SharedString>,
}

/// One node card.
#[derive(IntoElement)]
pub struct NodeCard {
    node: Node,
    sink: Option<EventSink>,
    details: Option<Disclosure>,
}

impl NodeCard {
    /// A read-only card for `node`.
    pub fn new(node: Node) -> Self {
        Self { node, sink: None, details: None }
    }

    /// Offers Cancel and Trust for a discovered node, emitting through `sink`.
    pub fn actions(mut self, sink: EventSink) -> Self {
        self.sink = Some(sink);
        self
    }

    /// The collapsed "Technical details" disclosure.
    pub fn details(mut self, disclosure: Disclosure) -> Self {
        self.details = Some(disclosure);
        self
    }
}

/// The node's endpoints without repeats, in the order they were listed.
fn unique(items: &[String]) -> Vec<&str> {
    let mut seen: Vec<&str> = Vec::new();
    for item in items {
        if !seen.contains(&item.as_str()) {
            seen.push(item);
        }
    }
    seen
}

impl RenderOnce for NodeCard {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let node = self.node;
        let fingerprint: SharedString =
            node.fingerprint.clone().map_or_else(|| tr("node.fingerprint_unknown"), Into::into);
        let endpoints = unique(&node.endpoints);
        let address = endpoints.first().map(|address| SharedString::from((*address).to_owned()));
        let join: SharedString =
            if endpoints.is_empty() { tr("status.none") } else { endpoints.join("\n").into() };
        let expiry: SharedString =
            node.expires_at.map_or_else(|| tr("grant.no_expiry"), |at| i18n::date_time(at).into());
        let pending = !node.confirmed;
        // Neutral, never a warning card: what the pin rests on.
        let status = match node.pin_source {
            PinSource::Manual => Some(tr("node.tofu_body")),
            PinSource::Invite => Some(tr("node.invite_body")),
            PinSource::Bootstrap => None,
        };
        let actions = self.sink.filter(|_| pending).map(|sink| {
            let confirm = sink.clone();
            let node_id = node.id.clone();
            let cancel_id = node.id.clone();
            h_flex()
                .gap(space(2.))
                .justify_row_end()
                .child(
                    // Cancel keeps what "Not now" did: the discovered node is
                    // dropped, so nothing is pinned and the address can be
                    // looked up again.
                    Button::new("node-cancel")
                        .design_secondary(cx)
                        .design_label(tr("common.cancel"))
                        .on_click(move |_, window, cx| {
                            sink.emit(
                                UiEventKind::NodeActionPressed {
                                    node_id: cancel_id.clone(),
                                    action: NodeAction::Remove,
                                },
                                window,
                                cx,
                            );
                        }),
                )
                .child(
                    Button::new("node-confirm")
                        .design_primary(cx)
                        .design_label(tr("node.trust"))
                        .on_click(move |_, window, cx| {
                            confirm.emit(
                                UiEventKind::ConfirmNodePressed { node_id: node_id.clone() },
                                window,
                                cx,
                            );
                        }),
                )
        });
        let details = self.details.map(|disclosure| {
            let Disclosure { open, toggle, socket } = disclosure;
            let mut list = DescriptionList::new()
                .columns(1)
                .item(tr("node.fingerprint"), fingerprint.clone(), 1)
                .item(tr("node.endpoints"), join.clone(), 1)
                .item(tr("node.capabilities"), crate::i18n::list(&node.capabilities), 1)
                .item(tr("node.expires"), expiry.clone(), 1)
                .item(tr("node.pin_source"), i18n::pin_source(node.pin_source), 1)
                .item(tr("node.health"), i18n::node_health(node.health), 1)
                .item(
                    tr("node.state"),
                    tr(if pending { "node.pending" } else { "node.confirmed" }),
                    1,
                );
            if let Some(socket) = socket.clone() {
                list = list.item(tr("entry.daemon.socket"), socket, 1);
            }
            v_flex()
                .id(domain_id("node-technical", &node.id))
                .gap(space(2.))
                .child(
                    h_flex().child(
                        Button::new("node-details-toggle")
                            .ghost()
                            .small()
                            .icon(if open { IconName::ChevronDown } else { forward_icon() })
                            .label(tr("node.details"))
                            .on_click(move |_, window, cx| toggle(window, cx)),
                    ),
                )
                .when(open, |column| column.child(list))
        });
        v_flex()
            .id(domain_id("node", &node.id))
            .gap(space(3.))
            .p(space(4.))
            .border_1()
            .border_color(cx.theme().border)
            .rounded(cx.theme().radius_lg)
            .child(
                div()
                    .id("node-title")
                    .role(gpui_kit::Role::Heading)
                    .text_size(text_subheading())
                    .child(tr("node.trust_title")),
            )
            .child(
                v_flex()
                    .gap(space(1.))
                    .child(
                        div()
                            .id("node-name")
                            .text_size(text_secondary())
                            .font_semibold()
                            .child(SharedString::from(node.id.to_string())),
                    )
                    .children(address.map(|address| {
                        div()
                            .id("node-address")
                            .text_size(text_caption())
                            .text_color(cx.theme().muted_foreground)
                            .child(address)
                    })),
            )
            .children(status.map(|status| {
                div()
                    .id("node-status")
                    .text_size(text_secondary())
                    .text_color(cx.theme().muted_foreground)
                    .child(status)
            }))
            .children(details)
            .children(actions)
    }
}
