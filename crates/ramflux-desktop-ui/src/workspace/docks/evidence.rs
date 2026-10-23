// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The evidence dock (read-only): protocol evidence for the current context,
//! summoned only when the user asks. Grant bindings, message franking, the
//! group epoch and device count, and the home node's fields.

use gpui_kit::component::description_list::DescriptionList;
use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, Context, Entity, IntoElement, Render, SharedString, Subscription, Window, div,
};
use ramflux_desktop_core::{ContextId, LockState, MomentKind, Node, Store, selectors};

use super::{empty_line, section};
use crate::app::{StoreEvent, StoreHandle};
use crate::i18n::{self, tr};

/// Franking records shown (newest first).
pub const FRANKING_SHOWN: usize = 6;

/// The evidence dock content.
pub struct EvidenceDock {
    store: Entity<StoreHandle>,
    context_id: Option<ContextId>,
    _subscription: Subscription,
}

impl EvidenceDock {
    /// A dock over `store`.
    pub fn new(store: Entity<StoreHandle>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.subscribe(&store, |_, _, event: &StoreEvent, cx| {
            if let StoreEvent::Changed(changes) = event
                && changes.touches_docks()
            {
                cx.notify();
            }
        });
        Self { store, context_id: None, _subscription: subscription }
    }

    /// Shows evidence for `context_id`.
    pub fn set_context(&mut self, context_id: Option<ContextId>, cx: &mut Context<Self>) {
        if self.context_id != context_id {
            self.context_id = context_id;
            cx.notify();
        }
    }
}

fn text(value: impl Into<SharedString>) -> SharedString {
    value.into()
}

fn grants(store: &Store, context: &ContextId) -> Vec<AnyElement> {
    selectors::timeline(store, context)
        .into_iter()
        .filter_map(|m| match &m.kind {
            MomentKind::Intervention(p) => store.grant(&p.grant_id),
            _ => None,
        })
        .map(|grant| {
            let expiry =
                grant.expiry.map_or_else(|| tr("grant.no_expiry"), |at| text(i18n::date_time(at)));
            DescriptionList::new()
                .columns(1)
                .item(tr("grant.scope"), text(grant.scope.clone()), 1)
                .item(tr("grant.args_hash"), text(grant.args_hash.clone()), 1)
                .item(
                    tr("grant.single_use"),
                    tr(if grant.single_use { "common.yes" } else { "common.no" }),
                    1,
                )
                .item(tr("grant.expiry"), expiry, 1)
                .item(tr("grant.state_label"), i18n::grant_state(grant.state), 1)
                .into_any_element()
        })
        .collect()
}

fn franking(store: &Store, context: &ContextId) -> Vec<(SharedString, SharedString)> {
    let mut records: Vec<(SharedString, SharedString)> = selectors::timeline(store, context)
        .into_iter()
        .filter_map(|m| match &m.kind {
            MomentKind::Message(p) => {
                p.franking.clone().map(|f| (text(i18n::date_time(m.at)), text(f)))
            }
            _ => None,
        })
        .collect();
    records.reverse();
    records.truncate(FRANKING_SHOWN);
    records
}

fn home_node(store: &Store) -> Option<&Node> {
    store
        .accounts()
        .find(|a| a.lock_state == LockState::Unlocked)
        .and_then(|a| a.node_id.as_ref())
        .and_then(|id| store.node(id))
        .or_else(|| store.nodes().find(|n| n.is_default))
}

fn node_fields(node: &Node) -> AnyElement {
    let expiry =
        node.expires_at.map_or_else(|| tr("grant.no_expiry"), |at| text(i18n::date_time(at)));
    DescriptionList::new()
        .columns(1)
        .item(tr("node.id"), text(node.id.to_string()), 1)
        .item(
            tr("node.fingerprint"),
            node.fingerprint.clone().map_or_else(|| tr("node.fingerprint_unknown"), text),
            1,
        )
        .item(tr("node.pin_source"), i18n::pin_source(node.pin_source), 1)
        .item(tr("node.expires"), expiry, 1)
        .item(tr("node.health"), i18n::node_health(node.health), 1)
        .item(tr("node.endpoints"), text(node.endpoints.join("\n")), 1)
        .into_any_element()
}

impl Render for EvidenceDock {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let store = self.store.read(cx).store();
        let context = self.context_id.as_ref().and_then(|id| store.context(id));
        let devices = store.devices().filter(|d| !d.revoked).count();
        let body = v_flex().id("dock-evidence").size_full().overflow_y_scroll();
        let Some(context) = context else {
            return body.child(empty_line("dock.evidence.none", cx));
        };
        let grants = grants(store, &context.id);
        let franking = franking(store, &context.id);
        let epoch = context.epoch.map_or_else(|| tr("status.none"), |e| text(e.to_string()));
        body.child(section("dock.evidence.grants", cx))
            .when(grants.is_empty(), |col| col.child(empty_line("dock.approvals.none", cx)))
            .children(grants.into_iter().map(|g| div().px_3().py_1().child(g)))
            .child(section("dock.evidence.franking", cx))
            .when(franking.is_empty(), |col| col.child(empty_line("dock.approvals.none", cx)))
            .children(franking.into_iter().map(|(at, value)| {
                v_flex()
                    .px_3()
                    .py_1()
                    .text_xs()
                    .child(div().text_color(cx.theme().muted_foreground).child(at))
                    .child(div().font_family(cx.theme().mono_font_family.clone()).child(value))
            }))
            .child(section("dock.evidence.group", cx))
            .child(div().px_3().py_1().child(
                DescriptionList::new().columns(1).item(tr("dock.evidence.epoch"), epoch, 1).item(
                    tr("dock.evidence.devices"),
                    text(devices.to_string()),
                    1,
                ),
            ))
            .child(section("dock.evidence.node", cx))
            .children(home_node(store).map(|node| div().px_3().py_1().child(node_fields(node))))
            .when(home_node(store).is_none(), |col| {
                col.child(empty_line("dock.evidence.no_node", cx))
            })
    }
}
