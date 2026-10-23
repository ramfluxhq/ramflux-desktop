// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Intervention moments: an approval bound to a grant.
//!
//! The body has three blocks: `summary`, `details` (a disclosure, view-local)
//! and `decision` (Approve / Deny). Approving a high-risk grant never commits
//! here: the root receives the request and policy opens the hold-to-confirm
//! strip above the composer.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::description_list::DescriptionList;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, IntoElement, SharedString, Window, div};
use ramflux_desktop_core::{
    AuditEntry, BlockId, Grant, GrantState, InterventionPayload, Lifecycle, Risk, Scope,
    UiEventKind,
};

use super::RowRefs;
use crate::bubble::EventSink;
use crate::i18n::{self, tr, tr_args};

/// Block id of the disclosure.
pub const DETAILS_BLOCK: &str = "details";
/// Block id of the Approve / Deny controls.
pub const DECISION_BLOCK: &str = "decision";

/// The body of an intervention moment.
#[derive(IntoElement)]
pub struct InterventionBody {
    payload: InterventionPayload,
    grant: Option<Grant>,
    audit: Option<AuditEntry>,
    proposer: SharedString,
    lifecycle: Lifecycle,
    sink: EventSink,
    expanded: bool,
}

impl InterventionBody {
    /// A body emitting through the moment-scoped `sink`.
    pub fn new(
        payload: InterventionPayload,
        refs: &RowRefs,
        lifecycle: Lifecycle,
        sink: EventSink,
    ) -> Self {
        let grant = refs.grants.get(&payload.grant_id).cloned();
        let audit = refs
            .audit
            .iter()
            .rev()
            .find(|entry| entry.subject == payload.grant_id.as_str())
            .cloned();
        Self {
            payload,
            grant,
            audit,
            proposer: SharedString::default(),
            lifecycle,
            sink,
            expanded: false,
        }
    }

    /// Display name of the participant who asked.
    pub fn proposed_by(mut self, proposer: SharedString) -> Self {
        self.proposer = proposer;
        self
    }

    /// Whether the details disclosure is open.
    pub fn expanded(mut self, expanded: bool) -> Self {
        self.expanded = expanded;
        self
    }

    fn is_open(&self) -> bool {
        self.lifecycle == Lifecycle::NeedsMe
            && self.grant.as_ref().is_none_or(|grant| grant.state == GrantState::Pending)
    }
}

impl RenderOnce for InterventionBody {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let risk_color = if self.payload.risk.requires_confirmation() {
            cx.theme().danger
        } else {
            cx.theme().muted_foreground
        };
        let open = self.is_open();
        let details_sink = self.sink.scoped(Scope::Block(BlockId::new(DETAILS_BLOCK)));
        let decision_sink = self.sink.scoped(Scope::Block(BlockId::new(DECISION_BLOCK)));
        let expanded = self.expanded;
        v_flex()
            .gap_2()
            .p_3()
            .border_1()
            .border_color(if open { risk_color } else { cx.theme().border })
            .rounded(cx.theme().radius_lg)
            .bg(cx.theme().group_box)
            .child(
                h_flex()
                    .gap_2()
                    .items_start()
                    .child(Icon::new(IconName::TriangleAlert).small().text_color(risk_color))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().foreground)
                                    .child(self.payload.summary.clone()),
                            )
                            .child(
                                h_flex()
                                    .gap_2()
                                    .text_xs()
                                    .child(div().text_color(risk_color).child(i18n::tr_args(
                                        "moment.intervention.risk",
                                        &[("risk", &i18n::risk(self.payload.risk))],
                                    )))
                                    .when_some(self.grant.as_ref(), |row, grant| {
                                        row.child(
                                            div()
                                                .font_family(cx.theme().mono_font_family.clone())
                                                .text_color(cx.theme().muted_foreground)
                                                .child(grant.scope.clone()),
                                        )
                                    }),
                            ),
                    )
                    .child(
                        Button::new("details")
                            .ghost()
                            .xsmall()
                            .icon(if expanded {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            })
                            .label(tr("moment.intervention.details"))
                            .on_click(move |_, window, cx| {
                                details_sink.emit(
                                    UiEventKind::ExpandToggled { expanded: !expanded },
                                    window,
                                    cx,
                                );
                            }),
                    ),
            )
            .when(expanded, |body| body.child(details(self.grant.as_ref(), self.payload.risk)))
            .child(if open {
                decision(decision_sink, self.payload.risk).into_any_element()
            } else {
                resolution(self.grant.as_ref(), self.lifecycle, cx).into_any_element()
            })
            .child(audit_line(&self.proposer, self.audit.as_ref(), cx))
    }
}

fn details(grant: Option<&Grant>, risk: Risk) -> impl IntoElement {
    let mut list = DescriptionList::new().columns(1).item(tr("grant.risk"), i18n::risk(risk), 1);
    if let Some(grant) = grant {
        let expiry: SharedString =
            grant.expiry.map_or_else(|| tr("grant.no_expiry"), |at| i18n::time_of_day(at).into());
        list = list
            .item(tr("grant.scope"), SharedString::from(grant.scope.clone()), 1)
            .item(tr("grant.args_hash"), SharedString::from(grant.args_hash.clone()), 1)
            .item(
                tr("grant.single_use"),
                tr(if grant.single_use { "common.yes" } else { "common.no" }),
                1,
            )
            .item(tr("grant.expiry"), expiry, 1);
    }
    list
}

fn decision(sink: EventSink, risk: Risk) -> impl IntoElement {
    let approve_sink = sink.clone();
    // Approving a high-risk grant needs the hold-to-confirm strip first, so
    // the label carries an ellipsis.
    let approve_label = tr(if risk.requires_confirmation() {
        "moment.intervention.approve_hold"
    } else {
        "moment.intervention.approve"
    });
    h_flex()
        .gap_2()
        .child(Button::new("approve").primary().small().label(approve_label).on_click(
            move |_, window, cx| {
                approve_sink.emit(UiEventKind::ApprovePressed, window, cx);
            },
        ))
        .child(
            Button::new("deny").outline().small().label(tr("moment.intervention.deny")).on_click(
                move |_, window, cx| {
                    sink.emit(UiEventKind::DenyPressed, window, cx);
                },
            ),
        )
}

fn resolution(grant: Option<&Grant>, lifecycle: Lifecycle, cx: &App) -> impl IntoElement {
    let key = match grant.map(|g| g.state) {
        Some(GrantState::Approved) => "moment.intervention.approved",
        Some(GrantState::Denied) => "moment.intervention.denied",
        Some(GrantState::Expired) => "moment.intervention.expired",
        Some(GrantState::Revoked) => "moment.intervention.revoked",
        Some(GrantState::Pending) | None => match lifecycle {
            Lifecycle::Dismissed => "moment.intervention.denied",
            _ => "moment.intervention.closed",
        },
    };
    div().text_xs().text_color(cx.theme().muted_foreground).child(tr(key))
}

/// Who proposed the request and, once decided with a hold, that the
/// attended human confirmed it (and when).
fn audit_line(proposer: &SharedString, audit: Option<&AuditEntry>, cx: &App) -> impl IntoElement {
    let confirmed = audit.filter(|entry| entry.human_confirmed).map(|entry| {
        tr_args("moment.intervention.confirmed_by", &[("time", &i18n::time_of_day(entry.at))])
    });
    h_flex()
        .id("intervention-audit")
        .gap_2()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(tr_args("moment.intervention.proposed_by", &[("name", proposer)]))
        .when_some(confirmed, |row, confirmed| row.child(confirmed))
        .test_support()
}
