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
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{ActiveTheme as _, IconName, Sizable as _, StyledExt as _, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, Entity, IntoElement, SharedString, Window, div};
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

/// The operator key field: the key the node's operator gave the user. It
/// sits under "Technical details" and pins the node as `operator` when it
/// equals the discovered key; for a node whose key changed it is the only
/// way on.
pub struct OperatorKey {
    /// The typed key.
    pub input: Entity<InputState>,
    /// Whether the field shows (a changed key always shows it).
    pub open: bool,
    /// Flips `open`.
    pub toggle: Toggle,
}

/// One node card.
#[derive(IntoElement)]
pub struct NodeCard {
    node: Node,
    sink: Option<EventSink>,
    details: Option<Disclosure>,
    operator: Option<OperatorKey>,
    dismiss: Option<Toggle>,
}

impl NodeCard {
    /// A read-only card for `node`.
    pub fn new(node: Node) -> Self {
        Self { node, sink: None, details: None, operator: None, dismiss: None }
    }

    /// Offers Cancel and Trust for a discovered node, emitting through `sink`.
    pub fn actions(mut self, sink: EventSink) -> Self {
        self.sink = Some(sink);
        self
    }

    /// What Cancel does for a node whose key changed: the pinned node is not
    /// removed (that would let the new key be trusted on first use), the page
    /// just stops showing the card.
    pub fn on_dismiss(mut self, dismiss: Toggle) -> Self {
        self.dismiss = Some(dismiss);
        self
    }

    /// The operator key field.
    pub fn operator(mut self, operator: OperatorKey) -> Self {
        self.operator = Some(operator);
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
        let changed = node.key_changed;
        // The typed operator key must be the discovered one, exactly.
        let operator_input = self.operator.as_ref().map(|operator| operator.input.clone());
        let typed: String = operator_input
            .as_ref()
            .map(|input| input.read(cx).value().trim().to_owned())
            .unwrap_or_default();
        let key_matches = !typed.is_empty() && node.public_key.as_deref() == Some(typed.as_str());
        let mismatch = !typed.is_empty() && !key_matches;
        // A changed key has no first-use way on: only the operator's key.
        // A node found over plain http is pinned with its operator's key only.
        let plain_http = pending && node.is_plain_http() && !key_matches;
        let trust_blocked = mismatch || (changed && !key_matches) || plain_http;
        // Neutral, never a warning card: what the pin rests on.
        let status = if changed {
            None
        } else if !pending {
            Some(tr("node.status_trusted"))
        } else {
            match node.pin_source {
                PinSource::Manual => Some(tr("node.tofu_body")),
                PinSource::Invite | PinSource::Operator => Some(tr("node.invite_body")),
                PinSource::Bootstrap => None,
            }
        };
        let danger = cx.theme().danger;
        let muted = cx.theme().muted_foreground;
        let field_input = operator_input.clone();
        let key_field = || {
            field_input.as_ref().map(|input| {
                v_flex()
                    .id("node-operator")
                    .gap(space(1.))
                    .child(
                        div()
                            .text_size(text_secondary())
                            .text_color(muted)
                            .child(tr("node.operator_instructions")),
                    )
                    .child(
                        Input::new(input)
                            .id("node-operator-key")
                            .aria_label(tr("node.operator_key")),
                    )
                    .children(mismatch.then(|| {
                        div()
                            .id("node-operator-mismatch")
                            .role(gpui_kit::Role::Alert)
                            .text_size(text_caption())
                            .text_color(danger)
                            .child(tr("node.operator_mismatch"))
                    }))
            })
        };
        let https_only = plain_http.then(|| {
            div()
                .id("node-https-only")
                .role(gpui_kit::Role::Alert)
                .text_size(text_caption())
                .text_color(danger)
                .child(tr("entry.node.https_only"))
        });
        let actions = self.sink.filter(|_| pending).map(|sink| {
            let confirm = sink.clone();
            let node_id = node.id.clone();
            let cancel_id = node.id.clone();
            let dismiss = self.dismiss.clone().filter(|_| changed);
            let trust = Button::new("node-confirm")
                .design_label(tr(if changed { "node.trust_new_key" } else { "node.trust" }))
                .on_click(move |_, window, cx| {
                    let key = operator_input
                        .as_ref()
                        .map(|input| input.read(cx).value().trim().to_owned())
                        .filter(|key| !key.is_empty());
                    confirm.emit(
                        UiEventKind::ConfirmNodePressed {
                            node_id: node_id.clone(),
                            operator_key: key,
                        },
                        window,
                        cx,
                    );
                });
            let trust = if changed { trust.design_danger(cx) } else { trust.design_primary(cx) }
                .design_disabled(trust_blocked, cx);
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
                            if let Some(dismiss) = &dismiss {
                                dismiss(window, cx);
                                return;
                            }
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
                .child(trust)
        });
        let operator_toggle = self.operator.as_ref().map(|o| (o.open, o.toggle.clone()));
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
            // Entering the operator's key is a text action in the details;
            // for a changed key the field is on the page already.
            let operator = (pending && !changed).then_some(operator_toggle).flatten();
            let operator_open = operator.as_ref().is_some_and(|(open, _)| *open);
            let operator_field = key_field().filter(|_| operator_open && !changed);
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
                .when(open, |column| {
                    column
                        .child(list)
                        .children(operator.map(|(_, toggle)| {
                            h_flex().child(
                                Button::new("node-operator-toggle")
                                    .ghost()
                                    .small()
                                    .label(tr("node.edit_operator_key"))
                                    .on_click(move |_, window, cx| toggle(window, cx)),
                            )
                        }))
                        .children(operator_field)
                })
        });
        let changed_warning = changed.then(|| {
            v_flex()
                .id("node-key-changed")
                .role(gpui_kit::Role::Alert)
                .gap(space(1.))
                .p(space(3.))
                .border_1()
                .border_color(danger)
                .rounded(cx.theme().radius)
                .child(
                    div()
                        .text_size(text_secondary())
                        .font_semibold()
                        .text_color(danger)
                        .child(tr("node.key_changed.title")),
                )
                .child(div().text_size(text_secondary()).child(tr("node.key_changed.body")))
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
            .children(https_only)
            .children(changed_warning)
            .children(key_field().filter(|_| changed))
            .children(details)
            .children(actions)
    }
}
