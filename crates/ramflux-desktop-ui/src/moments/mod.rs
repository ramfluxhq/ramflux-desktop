// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Inline moment rows of the conversation field.
//!
//! A [`MomentRow`] is a read-only presentation snapshot built from the store.
//! [`MomentView`] renders it with the row chrome and dispatches the body to
//! the renderer for its kind. In this build Message, System, Fallback and
//! Intervention have full renderers; every other kind renders a compact
//! placeholder naming the kind.
//!
//! Rows emit UI events through the moment-scoped [`EventSink`] they are given;
//! they never touch the store.

mod intervention;
mod message;
mod notice;
mod placeholder;

use gpui_kit::component::StyledExt as _;
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, IntoElement, SharedString, Window, div};
use ramflux_desktop_core::{Grant, Lifecycle, Moment, MomentKind, Store};

use crate::bubble::EventSink;
use crate::i18n;
use crate::ids::domain_id;

/// Presentation snapshot of one moment.
#[derive(Debug, Clone, PartialEq)]
pub struct MomentRow {
    /// The moment.
    pub moment: Moment,
    /// Display name of the origin participant.
    pub author: SharedString,
    /// The grant an intervention is bound to.
    pub grant: Option<Grant>,
}

impl MomentRow {
    /// Builds the snapshot for `moment`.
    pub fn build(store: &Store, moment: &Moment) -> Self {
        let author = store.participant(&moment.origin).map_or_else(
            || SharedString::from(moment.origin.to_string()),
            |p| p.display.clone().into(),
        );
        let grant = match &moment.kind {
            MomentKind::Intervention(payload) => store.grant(&payload.grant_id).cloned(),
            _ => None,
        };
        Self { moment: moment.clone(), author, grant }
    }
}

/// One rendered moment row.
#[derive(IntoElement)]
pub struct MomentView {
    row: MomentRow,
    sink: EventSink,
    expanded: bool,
    highlighted: bool,
}

impl MomentView {
    /// A row emitting through `sink`, which must already be scoped to the moment.
    pub fn new(row: MomentRow, sink: EventSink) -> Self {
        Self { row, sink, expanded: false, highlighted: false }
    }

    /// Whether the row's disclosure is expanded (view-local state).
    pub fn expanded(mut self, expanded: bool) -> Self {
        self.expanded = expanded;
        self
    }

    /// Whether the row is the target of "take me to what needs me".
    pub fn highlighted(mut self, highlighted: bool) -> Self {
        self.highlighted = highlighted;
        self
    }
}

impl RenderOnce for MomentView {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let moment = &self.row.moment;
        let body = match &moment.kind {
            MomentKind::Message(payload) => message::render(payload, cx).into_any_element(),
            MomentKind::System(payload) => notice::system(payload, cx).into_any_element(),
            MomentKind::Fallback(payload) => notice::fallback(payload, cx).into_any_element(),
            MomentKind::Intervention(payload) => intervention::InterventionBody::new(
                payload.clone(),
                self.row.grant.clone(),
                moment.lifecycle,
                self.sink.clone(),
            )
            .expanded(self.expanded)
            .into_any_element(),
            other => placeholder::render(other, cx).into_any_element(),
        };
        v_flex()
            .id(domain_id("moment", &moment.id))
            .w_full()
            .min_w_0()
            .gap_1()
            .px_2()
            .py_1()
            .rounded(cx.theme().radius)
            .when(self.highlighted, |row| row.bg(cx.theme().muted))
            .child(header(&self.row, cx))
            .child(body)
    }
}

fn header(row: &MomentRow, cx: &App) -> impl IntoElement {
    let moment = &row.moment;
    let marker = match moment.lifecycle {
        Lifecycle::Done => None,
        Lifecycle::NeedsMe => Some(cx.theme().warning),
        Lifecycle::Failed => Some(cx.theme().danger),
        Lifecycle::Pending | Lifecycle::InProgress | Lifecycle::Dismissed => {
            Some(cx.theme().muted_foreground)
        }
    };
    h_flex()
        .gap_2()
        .text_xs()
        .child(div().font_semibold().text_color(cx.theme().foreground).child(row.author.clone()))
        .child(div().text_color(cx.theme().muted_foreground).child(i18n::time_of_day(moment.at)))
        .when_some(marker, |header, color| {
            header.child(div().text_color(color).child(i18n::lifecycle(moment.lifecycle)))
        })
}
