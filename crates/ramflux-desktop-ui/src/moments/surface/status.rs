// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Surface notices: renderer verdicts and version states shown inline
//! (over a limit, out of date, out of sync with its author, changed after
//! review, a display-blocked block). They state what happened and what the user can do, in plain
//! words; protocol detail stays in the developer inspector.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::StyledExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Hsla, SharedString, div};
use std::collections::BTreeSet;

use ramflux_desktop_core::conformance::{LimitKind, SurfaceVerdict, check_surface};
use ramflux_desktop_core::{
    AgentSurfacePayload, BlockId, Moment, MomentId, MomentKind, RendererError, RendererErrorKind,
    Store, SurfaceHash, SurfaceState, Timestamp, UiEventKind, UpdateRequest,
};

use crate::bubble::EventSink;
use crate::i18n::{backend_text, tr, tr_args};

/// What the row knows about its surface version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceStatus {
    /// The renderer verdict on this version (carries its hash).
    pub verdict: SurfaceVerdict,
    /// The version number actions are bound to (with the hash).
    pub version: u64,
    /// A patch made against another version left it out of date.
    pub conflict: bool,
    /// The author closed it: a frozen record.
    pub closed: bool,
    /// The authority holding it knows it missed a change of the author's.
    pub out_of_sync: bool,
    /// The user's request for the author's current state, while relevant.
    pub update: Option<UpdateRequest>,
    /// Blocks whose text stream is open and has no text yet.
    pub waiting: BTreeSet<BlockId>,
    /// When the author last changed it (the authority's report).
    pub updated_at: Option<Timestamp>,
}

/// The renderer verdict on the surface a moment holds: the renderer's own
/// check, or a whole-surface fallback when the store refused it over a
/// renderer limit (it then keeps only the fallback text).
pub fn verdict(store: &Store, moment: &MomentId, surface: &AgentSurfacePayload) -> SurfaceVerdict {
    let unknown = SurfaceHash::of_version(0);
    let hash = store.surface_hash(moment).unwrap_or(&unknown);
    let mut verdict = check_surface(moment, surface, hash, super::registry::profile());
    if let Some(limit) = store.surface(moment).and_then(|r| r.over_limit()) {
        verdict.over_limit = Some(limit);
        verdict.blocks.clear();
        verdict.errors = vec![RendererError {
            moment_id: moment.clone(),
            surface_id: surface.surface_id.clone(),
            surface_hash: verdict.hash.clone(),
            block_id: None,
            block_kind: None,
            kind: RendererErrorKind::OverLimit(limit),
        }];
    }
    verdict
}

impl SurfaceStatus {
    /// The status of the surface `moment` holds in `store`, or `None` for
    /// other moments.
    pub fn of(store: &Store, moment: &Moment) -> Option<Self> {
        let MomentKind::AgentSurface(surface) = &moment.kind else {
            return None;
        };
        let verdict = verdict(store, &moment.id, surface);
        let record = store.surface(&moment.id);
        let waiting = record
            .map(|r| {
                r.streams
                    .iter()
                    .filter(|(_, s)| !s.finished && s.chars == 0)
                    .map(|(id, _)| id.clone())
                    .collect()
            })
            .unwrap_or_default();
        Some(Self {
            verdict,
            version: record.map_or(0, |r| r.version),
            conflict: store.surface_conflict(&moment.id),
            closed: record.is_some_and(|r| r.state == SurfaceState::Closed),
            out_of_sync: store.surface_out_of_sync(&moment.id),
            update: record.and_then(|r| r.update.clone()),
            waiting,
            updated_at: record.and_then(|r| r.updated_at),
        })
    }

    /// Why actions are inactive, if they are.
    pub fn blocked_reason(&self) -> Option<SharedString> {
        (self.conflict || self.out_of_sync).then(|| tr("surface.conflict.action"))
    }
}

fn notice(
    id: &'static str,
    icon: IconName,
    color: Hsla,
    title: SharedString,
    text: SharedString,
    cx: &App,
) -> AnyElement {
    h_flex()
        .id(id)
        .gap_2()
        .items_start()
        .p_2()
        .border_1()
        .border_color(color)
        .rounded(cx.theme().radius)
        .child(Icon::new(icon).small().text_color(color))
        .child(
            v_flex()
                .min_w_0()
                .gap_1()
                .child(div().text_sm().font_medium().child(title))
                .child(div().text_xs().text_color(cx.theme().muted_foreground).child(text)),
        )
        .test_support()
        .into_any_element()
}

fn limit_name(limit: LimitKind) -> SharedString {
    tr(match limit {
        LimitKind::Blocks => "surface.limit.blocks",
        LimitKind::Actions => "surface.limit.actions",
        LimitKind::TextLength => "surface.limit.text",
        LimitKind::DataBytes => "surface.limit.data",
        LimitKind::TextDelta => "surface.limit.delta",
        LimitKind::StreamLength => "surface.limit.stream",
    })
}

/// The whole-surface fallback note of a surface over a renderer limit.
pub fn over_limit(limit: LimitKind, cx: &App) -> AnyElement {
    notice(
        "surface-over-limit",
        IconName::TriangleAlert,
        cx.theme().warning,
        tr("surface.over_limit.title"),
        tr_args("surface.over_limit.text", &[("limit", &limit_name(limit))]),
        cx,
    )
}

/// The surface is out of date (a patch was made against another version).
pub fn conflict(cx: &App) -> AnyElement {
    notice(
        "surface-conflict",
        IconName::TriangleAlert,
        cx.theme().warning,
        tr("surface.conflict.title"),
        tr("surface.conflict.text"),
        cx,
    )
}

/// The surface is behind its author: what it shows may be old, and the
/// user can ask the author for the current state. The answer to that request
/// shows here too (sent, or why it could not be sent).
pub fn out_of_sync(update: Option<&UpdateRequest>, sink: &EventSink, cx: &App) -> AnyElement {
    let color = cx.theme().warning;
    let sent = matches!(update, Some(UpdateRequest::Sent));
    let answer: Option<SharedString> = match update {
        Some(UpdateRequest::Sent) => Some(tr("surface.out_of_sync.requested")),
        Some(UpdateRequest::Refused(reason)) => {
            Some(tr_args("surface.out_of_sync.refused", &[("reason", &backend_text(reason))]))
        }
        None => None,
    };
    let sink = sink.clone();
    let request = Button::new("surface-request-update")
        .small()
        .outline()
        .label(tr("surface.out_of_sync.request"))
        // Asked once; the author answers with its current state, which
        // ends this state. A refusal can be asked again.
        .disabled(sent)
        .on_click(move |_, window, cx| {
            sink.emit(UiEventKind::SurfaceUpdateRequested, window, cx);
        });
    h_flex()
        .id("surface-out-of-sync")
        .gap_2()
        .items_start()
        .p_2()
        .border_1()
        .border_color(color)
        .rounded(cx.theme().radius)
        .child(Icon::new(IconName::TriangleAlert).small().text_color(color))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(div().text_sm().font_medium().child(tr("surface.out_of_sync.title")))
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(tr("surface.out_of_sync.text")),
                )
                .child(h_flex().pt_1().child(request))
                .when_some(answer, |col, answer| {
                    col.child(
                        div()
                            .id("surface-update-answer")
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(answer)
                            .test_support(),
                    )
                }),
        )
        .test_support()
        .into_any_element()
}

/// The surface changed after the user reviewed it (a pending confirmation
/// was cancelled).
pub fn review_again(cx: &App) -> AnyElement {
    notice(
        "surface-review-again",
        IconName::Info,
        cx.theme().danger,
        tr("surface.review_again.title"),
        tr("surface.review_again.text"),
        cx,
    )
}

/// A closed surface: what it was, frozen.
pub fn closed(cx: &App) -> AnyElement {
    h_flex()
        .id("surface-closed")
        .gap_1()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(Icon::new(IconName::Check).xsmall())
        .child(tr("surface.closed"))
        .test_support()
        .into_any_element()
}

/// A display-blocked block: no data, only that something is hidden here.
pub fn redacted(cx: &App) -> AnyElement {
    h_flex()
        .id("block-redacted")
        .gap_1()
        .p_2()
        .border_1()
        .border_dashed()
        .border_color(cx.theme().border)
        .rounded(cx.theme().radius)
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(Icon::new(IconName::EyeOff).xsmall())
        .child(tr("surface.redacted"))
        .test_support()
        .into_any_element()
}
