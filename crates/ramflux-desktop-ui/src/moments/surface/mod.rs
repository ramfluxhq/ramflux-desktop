// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Agent surfaces: the declarative block catalog rendered by local, trusted
//! components (no peer code ever runs).
//!
//! Each block renderer receives its block data (untrusted) and a
//! block-scoped [`EventSink`]; it may only emit semantic events
//! (`ActionInvoked`, `ApprovePressed`, `FieldChanged`). Data a renderer
//! cannot read, and every unknown kind, renders as the Fallback block with
//! the surface's fallback text. Streaming text (`AppendText` patches) simply
//! re-renders the growing block. Long read-only surfaces collapse in place.
//!
//! - `content`: text, markdown, heading, key/value, metric, list, table,
//!   file preview and fallback.
//! - `feedback`: progress, log stream and callouts.
//! - `interactive`: form, input, select, action group and approval.

mod content;
mod feedback;
mod interactive;

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
use ramflux_desktop_core::{
    AgentSurfacePayload, Block, BlockId, BlockKind, Grant, GrantId, Scope, SurfaceAction,
    UiEventKind,
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
}

/// The body of an agent surface moment.
#[derive(IntoElement)]
pub struct SurfaceBody {
    payload: AgentSurfacePayload,
    grants: BTreeMap<GrantId, Grant>,
    sink: EventSink,
    expanded: bool,
    fields: Rc<BTreeMap<BlockId, String>>,
    inputs: Rc<BTreeMap<BlockId, Entity<InputState>>>,
}

impl SurfaceBody {
    /// A body emitting through the moment-scoped `sink`.
    pub fn new(
        payload: AgentSurfacePayload,
        refs: &RowRefs,
        sink: EventSink,
        state: &RowState,
    ) -> Self {
        Self {
            payload,
            grants: refs.grants.clone(),
            sink,
            expanded: state.expanded,
            fields: Rc::clone(&state.fields),
            inputs: Rc::clone(&state.inputs),
        }
    }
}

impl RenderOnce for SurfaceBody {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let surface_sink = self.sink.scoped(Scope::Surface(self.payload.surface_id.clone()));
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
        let fallback: SharedString = self.payload.fallback_text.clone().into();
        let blocks: Vec<AnyElement> = self.payload.blocks[..shown]
            .iter()
            .map(|block| {
                let context = BlockContext {
                    sink: surface_sink.scoped(Scope::Block(block.id.clone())),
                    actions: &self.payload.actions,
                    grants: &self.grants,
                    field: self.fields.get(&block.id),
                    input: self.inputs.get(&block.id),
                };
                div()
                    .id(domain_id("block", &block.id))
                    .w_full()
                    .min_w_0()
                    .child(render_block(block, &context, &fallback, cx))
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
        v_flex()
            .id("surface")
            .w_full()
            .gap_2()
            .p_3()
            .border_1()
            .border_color(cx.theme().border)
            .rounded(cx.theme().radius_lg)
            .bg(cx.theme().group_box)
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
                col.child(action_row("surface-actions", &loose, &surface_sink, cx))
            })
    }
}

/// Action ids listed by an action group block.
fn action_ids(block: &Block) -> Vec<&str> {
    block
        .data
        .get("actions")
        .and_then(|v| v.as_array())
        .map(|ids| ids.iter().filter_map(|id| id.as_str()).collect())
        .unwrap_or_default()
}

fn render_block(
    block: &Block,
    context: &BlockContext<'_>,
    fallback: &SharedString,
    cx: &App,
) -> AnyElement {
    let data = &block.data;
    let rendered = match block.kind {
        BlockKind::Text => content::text(data, cx),
        BlockKind::Markdown => content::markdown(&block.id, data, cx),
        BlockKind::Heading => content::heading(data, cx),
        BlockKind::KeyValue => content::key_value(data),
        BlockKind::Metric => content::metric(data, cx),
        BlockKind::List => content::list(data, cx),
        BlockKind::TableFallback => content::table(data, cx),
        BlockKind::FilePreview => content::file_preview(data, cx),
        BlockKind::Progress => feedback::progress(&block.id, data, cx),
        BlockKind::LogStream => feedback::log_stream(data, cx),
        BlockKind::Warning | BlockKind::Error | BlockKind::Success => {
            feedback::callout(&block.id, block.kind, data)
        }
        BlockKind::Form => interactive::form(data, cx),
        BlockKind::Input => interactive::input(data, context, cx),
        BlockKind::Select => interactive::select(data, context, cx),
        BlockKind::ActionGroup => interactive::action_group(&action_ids(block), context, cx),
        BlockKind::Approval => interactive::approval(data, context, cx),
        BlockKind::Fallback => None,
    };
    rendered.unwrap_or_else(|| content::fallback(fallback, cx))
}

/// A row of surface action buttons; disabled actions show their reason.
pub(crate) fn action_row(
    id: &'static str,
    actions: &[SurfaceAction],
    sink: &EventSink,
    cx: &App,
) -> AnyElement {
    let reasons: Vec<String> = actions.iter().filter_map(|a| a.disabled_reason.clone()).collect();
    let buttons = actions.iter().map(|action| {
        let sink = sink.clone();
        let action_id = action.id.clone();
        let label: SharedString = if action.risk.requires_confirmation() {
            tr_args("surface.action_hold", &[("label", &action.label)])
        } else {
            action.label.clone().into()
        };
        Button::new(domain_id("action", &action.id))
            .small()
            .outline()
            .label(label)
            .disabled(action.disabled_reason.is_some())
            .when_some(action.disabled_reason.clone(), |b, reason| b.tooltip(reason))
            .on_click(move |_, window, cx| {
                sink.emit(
                    UiEventKind::ActionInvoked {
                        action_id: action_id.clone(),
                        values: serde_json::Value::Null,
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
        .children(
            reasons.into_iter().map(|reason| {
                div().text_xs().text_color(cx.theme().muted_foreground).child(reason)
            }),
        )
        .into_any_element()
}
