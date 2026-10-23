// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Surface notices: renderer verdicts and version states shown inline
//! (over a limit, out of date, changed after review, a display-blocked
//! block). They state what happened and what the user can do, in plain
//! words; protocol detail stays in the developer inspector.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::StyledExt as _;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Hsla, SharedString, div};
use ramflux_desktop_core::conformance::{LimitKind, SurfaceVerdict, check_surface};
use ramflux_desktop_core::{Moment, MomentKind, Store};

use crate::i18n::{tr, tr_args};

/// What the row knows about its surface version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceStatus {
    /// The renderer verdict on this version (carries its hash).
    pub verdict: SurfaceVerdict,
    /// A patch made against another version left it out of date.
    pub conflict: bool,
}

impl SurfaceStatus {
    /// The status of the surface `moment` holds in `store`, or `None` for
    /// other moments.
    pub fn of(store: &Store, moment: &Moment) -> Option<Self> {
        let MomentKind::AgentSurface(surface) = &moment.kind else {
            return None;
        };
        let verdict = check_surface(&moment.id, surface, super::registry::profile());
        debug_assert_eq!(Some(&verdict.hash), store.surface_hash(&moment.id));
        Some(Self { verdict, conflict: store.surface_conflict(&moment.id) })
    }

    /// Why actions are inactive, if they are.
    pub fn blocked_reason(&self) -> Option<SharedString> {
        self.conflict.then(|| tr("surface.conflict.action"))
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
