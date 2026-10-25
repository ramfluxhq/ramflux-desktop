// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The approvals dock: pending requests (decided in place, through the same
//! policy as the inline intervention), decided grants (an approved, unused
//! grant can be revoked with a hold), standing approvals (revocable with a
//! hold) and the read-only audit trail.

use gpui_kit::component::StyledExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Context, Entity, IntoElement, Render, Subscription, Window, div};
use ramflux_desktop_core::{
    Actor, AuditEntry, Grant, GrantFilter, GrantState, Listing, MomentKind, Scope,
    StandingApproval, Store, UiEventKind, selectors,
};

use super::{empty_line, section};
use crate::app::{StoreEvent, StoreHandle};
use crate::bubble::EventSink;
use crate::i18n::{self, tr, tr_args};
use crate::ids::domain_id;

/// Audit records shown (newest first).
pub const AUDIT_SHOWN: usize = 20;

/// The approvals dock content.
pub struct ApprovalsDock {
    store: Entity<StoreHandle>,
    sink: EventSink,
    _subscription: Subscription,
}

impl ApprovalsDock {
    /// A dock over `store`; `sink` is the root sink (the dock adds its scopes).
    pub fn new(store: Entity<StoreHandle>, sink: EventSink, cx: &mut Context<Self>) -> Self {
        let subscription = cx.subscribe(&store, |_, _, event: &StoreEvent, cx| {
            if let StoreEvent::Changed(changes) = event
                && changes.touches_docks()
            {
                cx.notify();
            }
        });
        Self { store, sink, _subscription: subscription }
    }
}

fn grant_line(grant: &Grant, cx: &App) -> AnyElement {
    h_flex()
        .gap_2()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(i18n::risk(grant.risk))
        .child(
            div().font_family(cx.theme().mono_font_family.clone()).child(grant.args_hash.clone()),
        )
        .when_some(grant.expiry, |row, at| {
            row.child(tr_args("dock.approvals.expires", &[("time", &i18n::date_time(at))]))
        })
        .into_any_element()
}

/// A pending grant with Approve / Deny scoped to the moment that presents it.
fn pending_row(store: &Store, grant: &Grant, sink: &EventSink, cx: &App) -> AnyElement {
    let moment = selectors::presenting_moment(store, &grant.id);
    let summary = moment
        .and_then(|m| match &m.kind {
            MomentKind::Intervention(p) => Some(p.summary.clone()),
            _ => None,
        })
        .unwrap_or_else(|| grant.scope.clone());
    let decide = moment.map(|m| {
        let scoped =
            sink.scoped(Scope::Context(m.context_id.clone())).scoped(Scope::Moment(m.id.clone()));
        let block = match &m.kind {
            MomentKind::AgentSurface(s) => s
                .blocks
                .iter()
                .find(|b| {
                    b.data.get("grant_id").and_then(|v| v.as_str()) == Some(grant.id.as_str())
                })
                .map(|b| b.id.clone()),
            _ => None,
        };
        let scoped = match block {
            Some(block) => scoped.scoped(Scope::Block(block)),
            None => scoped,
        };
        let deny = scoped.clone();
        h_flex()
            .gap_2()
            .child(
                Button::new("approve")
                    .xsmall()
                    .primary()
                    .label(tr(if grant.risk.requires_confirmation() {
                        "moment.intervention.approve_hold"
                    } else {
                        "moment.intervention.approve"
                    }))
                    .on_click(move |_, window, cx| {
                        scoped.emit(UiEventKind::ApprovePressed, window, cx)
                    }),
            )
            .child(
                Button::new("deny")
                    .xsmall()
                    .outline()
                    .label(tr("moment.intervention.deny"))
                    .on_click(move |_, window, cx| deny.emit(UiEventKind::DenyPressed, window, cx)),
            )
    });
    // A failed grant is closed: its failure replaces the decision.
    let failure = grant.failure.map(|failure| crate::moments::grant_failure::render(failure, cx));
    let decide = if failure.is_some() { None } else { decide };
    v_flex()
        .id(domain_id("approvals-pending", &grant.id))
        .gap_1()
        .px_3()
        .py_1()
        .child(div().text_sm().font_medium().child(summary))
        .child(grant_line(grant, cx))
        .children(failure)
        .children(decide)
        .into_any_element()
}

fn grant_row(grant: &Grant, sink: &EventSink, cx: &App) -> AnyElement {
    let revoke = (grant.state == GrantState::Approved).then(|| {
        let sink = sink.clone();
        let grant_id = grant.id.clone();
        Button::new("revoke").xsmall().outline().label(tr("dock.approvals.revoke")).on_click(
            move |_, window, cx| {
                sink.emit(
                    UiEventKind::RevokeGrantPressed { grant_id: grant_id.clone() },
                    window,
                    cx,
                )
            },
        )
    });
    h_flex()
        .id(domain_id("approvals-grant", &grant.id))
        .gap_2()
        .px_3()
        .py_1()
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .text_sm()
                        .font_family(cx.theme().mono_font_family.clone())
                        .child(grant.scope.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(i18n::grant_state(grant.state)),
                )
                .when_some(grant.failure, |col, failure| {
                    col.child(
                        div()
                            .id("grant-failure-note")
                            .text_xs()
                            .text_color(cx.theme().danger)
                            .child(crate::moments::grant_failure::reason(failure)),
                    )
                }),
        )
        .children(revoke)
        .into_any_element()
}

fn standing_row(approval: &StandingApproval, sink: &EventSink, cx: &App) -> AnyElement {
    let revoke = (!approval.revoked).then(|| {
        let sink = sink.clone();
        let approval_id = approval.id.clone();
        Button::new("revoke").xsmall().outline().label(tr("dock.approvals.revoke")).on_click(
            move |_, window, cx| {
                sink.emit(
                    UiEventKind::RevokeStandingPressed { approval_id: approval_id.clone() },
                    window,
                    cx,
                );
            },
        )
    });
    let detail = tr_args(
        "dock.approvals.standing_detail",
        &[("risk", &i18n::risk(approval.max_risk)), ("uses", &approval.uses.to_string())],
    );
    h_flex()
        .id(domain_id("approvals-standing", &approval.id))
        .gap_2()
        .px_3()
        .py_1()
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .text_sm()
                        .font_family(cx.theme().mono_font_family.clone())
                        .child(approval.scope.clone()),
                )
                .child(
                    div().text_xs().text_color(cx.theme().muted_foreground).child(
                        if approval.revoked { tr("dock.approvals.revoked") } else { detail },
                    ),
                ),
        )
        .children(revoke)
        .into_any_element()
}

fn audit_row(store: &Store, entry: &AuditEntry, cx: &App) -> AnyElement {
    let who = match &entry.proposed_by {
        Actor::Human => tr("audit.you"),
        Actor::Agent(id) | Actor::Member(id) => {
            store.participant(id).map_or_else(|| id.short().into(), |p| p.display.clone().into())
        }
    };
    let held = if entry.human_confirmed { tr("audit.held") } else { tr("audit.not_held") };
    h_flex()
        .gap_2()
        .px_3()
        .py_1()
        .text_xs()
        .child(div().text_color(cx.theme().muted_foreground).child(i18n::date_time(entry.at)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .font_family(cx.theme().mono_font_family.clone())
                .truncate()
                .child(format!("{} {}", entry.action, entry.subject)),
        )
        .child(
            div()
                .text_color(cx.theme().muted_foreground)
                .child(tr_args("audit.by", &[("who", &who), ("held", &held)])),
        )
        .into_any_element()
}

impl Render for ApprovalsDock {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let store = self.store.read(cx).store();
        let sink = self.sink.scoped(Scope::Panel(super::DockTool::Approvals.scope_name()));
        let pending: Vec<Grant> =
            selectors::grants(store, GrantFilter::Pending).into_iter().cloned().collect();
        let decided: Vec<Grant> = selectors::grants(store, GrantFilter::All)
            .into_iter()
            .filter(|g| g.state != GrantState::Pending)
            .cloned()
            .collect();
        let standing: Vec<StandingApproval> = store.standing_approvals().cloned().collect();
        let audit: Vec<AuditEntry> =
            store.audit().iter().rev().take(AUDIT_SHOWN).cloned().collect();
        v_flex()
            .id("dock-approvals")
            .size_full()
            .overflow_y_scroll()
            .child(section("dock.approvals.pending", cx))
            .children(crate::list_state::placeholder(
                "approvals-list-state",
                crate::list_state::of(store, Listing::Approvals, pending.is_empty()),
                "dock.approvals.none_pending",
                cx,
            ))
            .children(pending.iter().map(|g| pending_row(store, g, &sink, cx)))
            .child(section("dock.approvals.grants", cx))
            .when(decided.is_empty(), |col| col.child(empty_line("dock.approvals.none", cx)))
            .children(decided.iter().map(|g| grant_row(g, &sink, cx)))
            .child(section("dock.approvals.standing", cx))
            .when(standing.is_empty(), |col| col.child(empty_line("dock.approvals.none", cx)))
            .children(standing.iter().map(|a| standing_row(a, &sink, cx)))
            .child(section("dock.approvals.audit", cx))
            .when(audit.is_empty(), |col| col.child(empty_line("dock.approvals.none", cx)))
            .children(audit.iter().map(|e| audit_row(store, e, cx)))
    }
}
