// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Agent surfaces: the declarative block catalog rendered by local, trusted
//! components (no peer code ever runs).
//!
//! Each block renderer receives its block data (untrusted) and a
//! block-scoped [`EventSink`]; it may only emit semantic events
//! (`ActionInvoked`, `ApprovePressed`, `FieldChanged`). Blocks draw only
//! through the [`registry`]; the row's renderer verdict (computed in core
//! against the registry's profile) decides what falls back: a surface over
//! a limit shows only its fallback text and no actions, and an unsupported,
//! malformed or display-blocked block shows a block fallback. Data a
//! renderer still cannot read falls back the same way. Every action carries
//! the surface version it was shown on (hash and version number). Streaming
//! text re-renders the growing block in place; an open stream that has no
//! text yet holds one line, and the "writing" mark sits in the frame's
//! corner, so a stream starting or ending never shifts the layout. A closed
//! surface is frozen: its fallback text, with the final view behind a
//! disclosure and no actions. Long read-only surfaces collapse in place.
//!
//! - `content`: text, markdown, heading, key/value, metric, list, table,
//!   file preview and fallback.
//! - `feedback`: progress, log stream and callouts.
//! - `interactive`: form, input, select, action group and approval.
//! - [`registry`]: the block renderers and the renderer profile.
//! - `status`: verdict and version notices.

mod content;
mod feedback;
mod interactive;
pub mod registry;
mod status;

pub use status::{SurfaceStatus, verdict};

use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::InputState;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Entity, IntoElement, SharedString, Window, div};
use ramflux_desktop_core::conformance::BlockIssue;
use ramflux_desktop_core::{
    AgentSurfacePayload, Block, BlockId, BlockKind, Grant, GrantId, Scope, SurfaceAction,
    SurfaceHash, UiEventKind,
};

use super::{RowRefs, RowState};
use crate::bubble::EventSink;
use crate::i18n::{tr, tr_args};
use crate::ids::domain_id;

/// Blocks shown while a long surface is collapsed.
pub const COLLAPSED_BLOCKS: usize = 4;
/// Block scope of the expand / collapse control.
pub const DISCLOSURE_BLOCK: &str = "disclosure";

/// What a block renderer may read.
pub struct BlockContext<'a> {
    /// Block-scoped sink.
    pub sink: EventSink,
    /// Surface actions (for action groups).
    pub actions: &'a [SurfaceAction],
    /// Grants referenced by approval blocks.
    pub grants: &'a BTreeMap<GrantId, Grant>,
    /// View-local selected value of a select block.
    pub field: Option<&'a String>,
    /// View-local text input of an input block.
    pub input: Option<&'a Entity<InputState>>,
    /// The surface's fallback text (for fallback blocks).
    pub fallback: &'a SharedString,
    /// The surface version actions are bound to.
    pub hash: &'a SurfaceHash,
    /// Its version number.
    pub version: u64,
    /// Why actions are inactive, if they are.
    pub blocked: Option<&'a SharedString>,
}

/// The body of an agent surface moment.
#[derive(IntoElement)]
pub struct SurfaceBody {
    payload: AgentSurfacePayload,
    status: SurfaceStatus,
    grants: BTreeMap<GrantId, Grant>,
    sink: EventSink,
    expanded: bool,
    review_again: bool,
    fields: Rc<BTreeMap<BlockId, String>>,
    inputs: Rc<BTreeMap<BlockId, Entity<InputState>>>,
}

impl SurfaceBody {
    /// A body emitting through the moment-scoped `sink`.
    pub fn new(
        payload: AgentSurfacePayload,
        status: SurfaceStatus,
        refs: &RowRefs,
        sink: EventSink,
        state: &RowState,
    ) -> Self {
        Self {
            payload,
            status,
            grants: refs.grants.clone(),
            sink,
            expanded: state.expanded,
            review_again: state.review_again,
            fields: Rc::clone(&state.fields),
            inputs: Rc::clone(&state.inputs),
        }
    }
}

impl RenderOnce for SurfaceBody {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let surface_sink = self.sink.scoped(Scope::Surface(self.payload.surface_id.clone()));
        let fallback: SharedString = self.payload.fallback_text.clone().into();
        let frame = v_flex()
            .id("surface")
            .relative()
            .w_full()
            .gap_2()
            .p_3()
            .border_1()
            .border_color(cx.theme().border)
            .rounded(cx.theme().radius_lg)
            .bg(cx.theme().group_box)
            .when(self.review_again, |col| col.child(status::review_again(cx)));
        if let Some(limit) = self.status.verdict.over_limit {
            // Never a partial rendering, and nothing to act on.
            return frame
                .child(status::over_limit(limit, cx))
                .child(content::fallback(&fallback, cx));
        }
        let disclosure_sink = surface_sink.scoped(Scope::Block(BlockId::new(DISCLOSURE_BLOCK)));
        let expanded = self.expanded;
        if self.status.closed {
            // Frozen: what it says in one line; the final view on request.
            let blocks = if expanded { self.blocks(&surface_sink, cx) } else { Vec::new() };
            return frame
                .child(status::closed(cx))
                .child(
                    div()
                        .id("surface-closed-text")
                        .text_sm()
                        .text_color(cx.theme().foreground)
                        .child(fallback.clone())
                        .test_support(),
                )
                .children(blocks)
                .when(!self.payload.blocks.is_empty(), |col| {
                    col.child(disclosure(
                        expanded,
                        tr(if expanded { "surface.hide_final" } else { "surface.show_final" }),
                        disclosure_sink,
                    ))
                });
        }
        let hash = self.status.verdict.hash.clone();
        let blocked = self.status.blocked_reason();
        let grouped: BTreeSet<&str> = self
            .payload
            .blocks
            .iter()
            .filter(|b| b.kind == BlockKind::ActionGroup)
            .flat_map(|b| action_ids(b))
            .collect();
        let total = self.payload.blocks.len();
        let blocks = self.blocks(&surface_sink, cx);
        let collapsible = blocks.len() < total;
        let loose: Vec<SurfaceAction> = self
            .payload
            .actions
            .iter()
            .filter(|a| !grouped.contains(a.id.as_str()))
            .cloned()
            .collect();
        frame
            .when(self.payload.streaming, |col| {
                // Out of the flow: starting or ending a stream moves nothing.
                col.child(
                    h_flex()
                        .id("surface-streaming")
                        .absolute()
                        .top_2()
                        .right_3()
                        .gap_1()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(Icon::new(IconName::LoaderCircle).xsmall())
                        .child(tr("surface.streaming"))
                        .test_support(),
                )
            })
            .when(self.status.conflict, |col| col.child(status::conflict(cx)))
            .children(blocks)
            .when(collapsible || (expanded && total > COLLAPSED_BLOCKS + 1), |col| {
                let label = if expanded {
                    tr("surface.show_less")
                } else {
                    tr_args("surface.show_all", &[("count", &total.to_string())])
                };
                col.child(disclosure(expanded, label, disclosure_sink))
            })
            .when(!loose.is_empty(), |col| {
                let row = ActionRow {
                    hash: &hash,
                    version: self.status.version,
                    blocked: blocked.as_ref(),
                };
                col.child(row.render("surface-actions", &loose, &surface_sink, cx))
            })
    }
}

impl SurfaceBody {
    /// The blocks shown now: all of them, or the first few of a long
    /// read-only surface while it is collapsed.
    fn blocks(&self, surface_sink: &EventSink, cx: &App) -> Vec<AnyElement> {
        let fallback: SharedString = self.payload.fallback_text.clone().into();
        let hash = self.status.verdict.hash.clone();
        let blocked = self.status.blocked_reason();
        let total = self.payload.blocks.len();
        // Read-only surfaces collapse; a surface that asks for input never
        // hides its controls, and a closed one shows everything once opened.
        let interactive = self.payload.blocks.iter().any(|b| {
            matches!(
                b.kind,
                BlockKind::Form | BlockKind::Input | BlockKind::Select | BlockKind::Approval
            )
        });
        let collapsible = total > COLLAPSED_BLOCKS + 1 && !interactive && !self.status.closed;
        let shown = if collapsible && !self.expanded { COLLAPSED_BLOCKS } else { total };
        self.payload.blocks[..shown]
            .iter()
            .map(|block| {
                let context = BlockContext {
                    sink: surface_sink.scoped(Scope::Block(block.id.clone())),
                    actions: if self.status.closed { &[] } else { &self.payload.actions },
                    grants: &self.grants,
                    field: self.fields.get(&block.id),
                    input: self.inputs.get(&block.id),
                    fallback: &fallback,
                    hash: &hash,
                    version: self.status.version,
                    blocked: blocked.as_ref(),
                };
                let issue = self.status.verdict.blocks.get(&block.id).copied();
                let element = if self.status.waiting.contains(&block.id) {
                    waiting_line(cx)
                } else {
                    render_block(block, issue, &context, cx)
                };
                div()
                    .id(domain_id("block", &block.id))
                    .w_full()
                    .min_w_0()
                    .child(element)
                    .test_support()
                    .into_any_element()
            })
            .collect()
    }
}

/// An open stream with no text yet: one line of the body text's height, so
/// the first text replaces it without moving anything.
fn waiting_line(cx: &App) -> AnyElement {
    div()
        .id("block-waiting")
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(tr("surface.waiting"))
        .test_support()
        .into_any_element()
}

/// The expand / collapse control of a surface.
fn disclosure(expanded: bool, label: SharedString, sink: EventSink) -> AnyElement {
    Button::new("surface-disclosure")
        .ghost()
        .xsmall()
        .icon(if expanded { IconName::ChevronDown } else { IconName::ChevronRight })
        .label(label)
        .on_click(move |_, window, cx| {
            sink.emit(UiEventKind::ExpandToggled { expanded: !expanded }, window, cx);
        })
        .into_any_element()
}

/// Action ids listed by an action group block.
pub(crate) fn action_ids(block: &Block) -> Vec<&str> {
    block
        .data
        .get("actions")
        .and_then(|v| v.as_array())
        .map(|ids| ids.iter().filter_map(|id| id.as_str()).collect())
        .unwrap_or_default()
}

fn render_block(
    block: &Block,
    issue: Option<BlockIssue>,
    context: &BlockContext<'_>,
    cx: &App,
) -> AnyElement {
    match issue {
        Some(BlockIssue::Redacted) => status::redacted(cx),
        Some(BlockIssue::Unsupported | BlockIssue::Schema) => {
            content::fallback(context.fallback, cx)
        }
        None => registry::render(block, context, cx)
            .unwrap_or_else(|| content::fallback(context.fallback, cx)),
    }
}

/// What surface action buttons are bound to.
pub(crate) struct ActionRow<'a> {
    /// The surface version shown.
    pub hash: &'a SurfaceHash,
    /// Its version number.
    pub version: u64,
    /// Why every action is inactive, if it is.
    pub blocked: Option<&'a SharedString>,
}

impl ActionRow<'_> {
    /// A row of surface action buttons; disabled actions show their reason.
    pub fn render(
        &self,
        id: &'static str,
        actions: &[SurfaceAction],
        sink: &EventSink,
        cx: &App,
    ) -> AnyElement {
        let mut reasons: Vec<SharedString> =
            actions.iter().filter_map(|a| a.disabled_reason.clone().map(Into::into)).collect();
        if let Some(blocked) = self.blocked {
            reasons = vec![blocked.clone()];
        }
        let buttons = actions.iter().map(|action| {
            let sink = sink.clone();
            let action_id = action.id.clone();
            let hash = self.hash.clone();
            let version = self.version;
            let label: SharedString = if action.risk.requires_confirmation() {
                tr_args("surface.action_hold", &[("label", &action.label)])
            } else {
                action.label.clone().into()
            };
            let reason: Option<SharedString> =
                self.blocked.cloned().or_else(|| action.disabled_reason.clone().map(Into::into));
            Button::new(domain_id("action", &action.id))
                .small()
                .outline()
                .label(label)
                .disabled(reason.is_some())
                .when_some(reason, |b, reason| b.tooltip(reason))
                .on_click(move |_, window, cx| {
                    sink.emit(
                        UiEventKind::ActionInvoked {
                            action_id: action_id.clone(),
                            values: serde_json::Value::Null,
                            surface_hash: hash.clone(),
                            version,
                        },
                        window,
                        cx,
                    );
                })
        });
        v_flex()
            .id(id)
            .gap_1()
            .child(h_flex().flex_wrap().gap_2().children(buttons))
            .children(reasons.into_iter().map(|reason| {
                div().text_xs().text_color(cx.theme().muted_foreground).child(reason)
            }))
            .into_any_element()
    }
}
