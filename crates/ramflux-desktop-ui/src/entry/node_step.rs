// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The node step: discovered nodes waiting for a decision as full NODE-01
//! cards, the add-node form, then the nodes already confirmed as compact
//! rows. The confirm strip for trust-on-first-use pins is the page's footer.

use crate::style::DesignButton as _;
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::Button;
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::input::Input;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{Context, IntoElement, Role, SharedString, Window, div};
use ramflux_desktop_core::{ListState, Listing, Node, UiEventKind, methods};
use ramflux_desktop_navigation::direction::h_flex;
use ramflux_desktop_navigation::scale::{space, text_caption, text_secondary};
use std::rc::Rc;

use super::node_card::{Disclosure, OperatorKey};
use super::{EntryPage, NodeCard};
use crate::i18n::tr;

impl EntryPage {
    /// Submits the typed node address for discovery and clears the input.
    pub fn add_node(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let url = self.inputs.node_url.read(cx).value().trim().to_owned();
        if url.is_empty() {
            return;
        }
        self.node_looked_up = Some(url.clone());
        self.changed_dismissed = false;
        self.inputs.node_url.update(cx, |input, cx| input.set_value("", window, cx));
        self.node_lookup = Some(self.store.read(cx).store().failures_total());
        self.sink.emit(UiEventKind::AddNodeRequested { url }, window, cx);
    }

    /// Why the last node lookup failed, translated (an unreachable host on
    /// macOS names the Local Network permission).
    fn node_lookup_failure(&self, cx: &Context<Self>) -> Option<SharedString> {
        let since = self.node_lookup?;
        let store = self.store.read(cx).store();
        store
            .failures_since(since)
            .iter()
            .rev()
            .find(|f| f.method == methods::NODE_ADD || f.method == methods::NODE_PROBE)
            .map(|f| crate::i18n::backend_text(&f.message))
    }

    pub(super) fn render_node_step(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let lookup_failure = self.node_lookup_failure(cx);
        let store = self.store.read(cx).store();
        let (pending_nodes, known): (Vec<Node>, Vec<Node>) =
            store.nodes().cloned().partition(|n| !n.confirmed);
        // A cancelled key change leaves the pinned node as it was.
        let pending_nodes: Vec<Node> = pending_nodes
            .into_iter()
            .filter(|n| !(n.key_changed && self.changed_dismissed))
            .collect();
        // Loading until the source listed the nodes; then an empty list is
        // an explicit empty state, never a perpetual spinner.
        let state = crate::list_state::of(
            store,
            Listing::Nodes,
            pending_nodes.is_empty() && known.is_empty(),
        );
        let pending = !pending_nodes.is_empty();
        let usable = known.iter().any(|n| n.enabled);
        let daemon = store.daemon().map(|info| {
            crate::i18n::tr_args(
                "backend.daemon.socket",
                &[("path", &info.socket_path), ("accounts", &info.open_accounts.to_string())],
            )
        });
        let details_open = self.node_details;
        let toggle = {
            let this = cx.entity().downgrade();
            Rc::new(move |_: &mut Window, cx: &mut gpui_kit::App| {
                this.update(cx, |page, cx| {
                    page.node_details = !page.node_details;
                    cx.notify();
                })
                .ok();
            })
        };
        // The socket path is a technical detail: it rides in the first
        // card's disclosure, or in the page's own when no node waits.
        let socket_line = daemon.clone();
        let operator_toggle = {
            let this = cx.entity().downgrade();
            Rc::new(move |_: &mut Window, cx: &mut gpui_kit::App| {
                this.update(cx, |page, cx| {
                    page.node_operator = !page.node_operator;
                    cx.notify();
                })
                .ok();
            })
        };
        let dismiss = {
            let this = cx.entity().downgrade();
            Rc::new(move |_: &mut Window, cx: &mut gpui_kit::App| {
                this.update(cx, |page, cx| {
                    page.changed_dismissed = true;
                    cx.notify();
                })
                .ok();
            })
        };
        let operator_open = self.node_operator;
        let operator_input = self.inputs.operator_key.clone();
        let cards = pending_nodes.into_iter().enumerate().map(|(ix, node)| {
            NodeCard::new(node)
                .actions(self.sink.clone())
                .on_dismiss(dismiss.clone())
                .details(Disclosure {
                    open: details_open,
                    toggle: toggle.clone(),
                    socket: socket_line.clone().filter(|_| ix == 0),
                })
                .operator(OperatorKey {
                    input: operator_input.clone(),
                    open: operator_open,
                    toggle: operator_toggle.clone(),
                })
        });
        // The address just looked up is a node pinned with this key already:
        // nothing waits for a decision, so the page says so.
        let already_trusted = self.node_looked_up.as_deref().is_some_and(|url| {
            let url = origin_of(url);
            known.iter().any(|n| {
                n.id.as_str().eq_ignore_ascii_case(&url)
                    || n.endpoints.iter().any(|e| origin_of(e) == url)
            })
        });
        let page_details = daemon.clone().filter(|_| !pending).map(|line| {
            v_flex()
                .id("entry-node-technical")
                .gap(space(2.))
                .child(
                    h_flex().child(
                        Button::new("entry-node-details-toggle")
                            .ghost()
                            .small()
                            .icon(if details_open {
                                IconName::ChevronDown
                            } else {
                                ramflux_desktop_navigation::direction::forward_icon()
                            })
                            .label(tr("node.details"))
                            .on_click({
                                let toggle = toggle.clone();
                                move |_, window, cx| toggle(window, cx)
                            }),
                    ),
                )
                .when(details_open, |column| {
                    column.child(
                        div()
                            .id("entry-daemon-info")
                            .text_size(text_caption())
                            .text_color(cx.theme().muted_foreground)
                            .child(line)
                            .test_support(),
                    )
                })
        });
        let muted = cx.theme().muted_foreground;
        let rows = known.into_iter().map(move |node| {
            h_flex()
                .id(crate::ids::domain_id("node-row", &node.id))
                .gap(space(2.))
                .text_size(text_secondary())
                .child(div().child(node.id.to_string()))
                .child(div().text_size(text_caption()).text_color(muted).child(crate::i18n::join(
                    &crate::i18n::pin_source(node.pin_source),
                    &crate::i18n::node_health(node.health),
                )))
        });
        v_flex()
            .id("entry-node")
            .gap(space(3.))
            .child(
                div()
                    .text_size(text_secondary())
                    .text_color(cx.theme().muted_foreground)
                    .child(tr("entry.node.intro")),
            )
            .when(state == ListState::Loading, |step| {
                step.child(
                    h_flex()
                        .id("entry-node-loading")
                        .role(Role::Status)
                        .aria_label(tr("entry.node.waiting"))
                        .test_support()
                        .gap(space(3.))
                        .child(Spinner::new())
                        .child(div().text_size(text_secondary()).child(tr("entry.node.waiting"))),
                )
            })
            .when(state == ListState::Empty, |step| {
                step.child(
                    h_flex()
                        .id("entry-node-empty")
                        .role(Role::Status)
                        .aria_label(tr("entry.node.none"))
                        .test_support()
                        .gap(space(3.))
                        .p(space(3.))
                        .border_1()
                        .border_color(cx.theme().border)
                        .rounded(cx.theme().radius_lg)
                        .child(Icon::new(IconName::Info).text_color(cx.theme().muted_foreground))
                        .child(div().text_size(text_secondary()).child(tr("entry.node.none"))),
                )
            })
            .children(cards)
            .child(
                v_flex()
                    .gap(space(2.))
                    .pt(space(2.))
                    .child(div().text_size(text_secondary()).child(tr("entry.node.add_title")))
                    .child(
                        h_flex()
                            .gap(space(2.))
                            .child(
                                Input::new(&self.inputs.node_url)
                                    .id("node-url")
                                    .aria_label(tr("entry.node.add_title"))
                                    .flex_1(),
                            )
                            .child(
                                Button::new("node-add")
                                    .design_secondary(cx)
                                    .icon(IconName::Search)
                                    .design_label(tr("entry.node.add"))
                                    .on_click(
                                        cx.listener(|this, _, window, cx| {
                                            this.add_node(window, cx)
                                        }),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .text_size(text_caption())
                            .text_color(cx.theme().muted_foreground)
                            .child(tr("entry.node.add_help")),
                    )
                    .children(lookup_failure.map(|reason| {
                        div()
                            .id("entry-node-add-failure")
                            .role(Role::Alert)
                            .aria_label(reason.clone())
                            .test_support()
                            .child(
                                Alert::error("entry-node-add-failure-alert", reason)
                                    .title(tr("entry.node.add_failed")),
                            )
                    })),
            )
            .when(already_trusted && !pending, |step| {
                step.child(
                    div()
                        .id("entry-node-already-trusted")
                        .role(Role::Status)
                        .text_size(text_secondary())
                        .text_color(cx.theme().muted_foreground)
                        .child(tr("node.status_trusted"))
                        .test_support(),
                )
            })
            .when(usable, |step| {
                step.child(
                    div().pt(space(2.)).text_size(text_secondary()).child(tr("entry.node.known")),
                )
                .children(rows)
            })
            .children(page_details)
            .when(usable && !pending && self.review_nodes, |step| {
                step.child(
                    Button::new("entry-continue")
                        .design_primary(cx)
                        .small()
                        .design_label(tr("entry.node.continue"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.review_nodes = false;
                            cx.notify();
                        })),
                )
            })
    }
}

/// An address without its scheme, path or case, to tell whether two name the
/// same node.
fn origin_of(address: &str) -> String {
    let address = address.trim().to_ascii_lowercase();
    let address = address.split_once("://").map_or(address.as_str(), |(_, rest)| rest);
    address.split('/').next().unwrap_or_default().to_owned()
}
