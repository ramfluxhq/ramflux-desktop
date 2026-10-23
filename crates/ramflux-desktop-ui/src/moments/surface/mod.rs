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
//! the surface version it was shown on. Streaming text (`AppendText`
//! patches) re-renders the growing block. Long read-only surfaces collapse
//! in place.
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

pub use status::SurfaceStatus;

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
        // Read-only surfaces collapse; a surface that asks for input never
        // hides its controls.
        let interactive = self.payload.blocks.iter().any(|b| {
            matches!(
                b.kind,
                BlockKind::Form | BlockKind::Input | BlockKind::Select | BlockKind::Approval
            )
        });
        let collapsible = total > COLLAPSED_BLOCKS + 1 && !interactive;
        let shown = if collapsible && !self.expanded { COLLAPSED_BLOCKS } else { total };
        let blocks: Vec<AnyElement> = self.payload.blocks[..shown]
            .iter()
            .map(|block| {
                let context = BlockContext {
                    sink: surface_sink.scoped(Scope::Block(block.id.clone())),
                    actions: &self.payload.actions,
                    grants: &self.grants,
                    field: self.fields.get(&block.id),
                    input: self.inputs.get(&block.id),
                    fallback: &fallback,
                    hash: &hash,
                    blocked: blocked.as_ref(),
                };
                let issue = self.status.verdict.blocks.get(&block.id).copied();
                div()
                    .id(domain_id("block", &block.id))
                    .w_full()
                    .min_w_0()
                    .child(render_block(block, issue, &context, cx))
                    .test_support()
                    .into_any_element()
            })
            .collect();
        let loose: Vec<SurfaceAction> = self
            .payload
            .actions
            .iter()
            .filter(|a| !grouped.contains(a.id.as_str()))
            .cloned()
            .collect();
        let disclosure_sink = surface_sink.scoped(Scope::Block(BlockId::new(DISCLOSURE_BLOCK)));
        let expanded = self.expanded;
        frame
            .when(self.status.conflict, |col| col.child(status::conflict(cx)))
            .children(blocks)
            .when(self.payload.streaming, |col| {
                col.child(
                    h_flex()
                        .id("surface-streaming")
                        .gap_1()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(Icon::new(IconName::LoaderCircle).xsmall())
                        .child(tr("surface.streaming"))
                        .test_support(),
                )
            })
            .when(collapsible, |col| {
                let label = if expanded {
                    tr("surface.show_less")
                } else {
                    tr_args("surface.show_all", &[("count", &total.to_string())])
                };
                col.child(
                    Button::new("surface-disclosure")
                        .ghost()
                        .xsmall()
                        .icon(if expanded { IconName::ChevronDown } else { IconName::ChevronRight })
                        .label(label)
                        .on_click(move |_, window, cx| {
                            disclosure_sink.emit(
                                UiEventKind::ExpandToggled { expanded: !expanded },
                                window,
                                cx,
                            );
                        }),
                )
            })
            .when(!loose.is_empty(), |col| {
                let row = ActionRow { hash: &hash, blocked: blocked.as_ref() };
                col.child(row.render("surface-actions", &loose, &surface_sink, cx))
            })
    }
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
