// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! View-local state of the conversation field's rows (rule R4): which rows
//! are expanded, surface form fields, conflict compare states. Nothing here
//! reaches the store; it is dropped when the field shows another context.

use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use gpui_kit::component::diff::{DiffFile, DiffMode, DiffState};
use gpui_kit::component::input::InputState;
use gpui_kit::{App, AppContext as _, Entity, Window};
use ramflux_desktop_core::{BlockId, BlockKind, JsonValue, MomentId, MomentKind};

use crate::moments::{MomentRow, RowState};

/// Per-row view-local state. Cloning is cheap (shared maps of handles),
/// so the row renderer gets a snapshot per frame.
#[derive(Clone, Default)]
pub struct ViewState {
    expanded: BTreeSet<MomentId>,
    fields: BTreeMap<MomentId, Rc<BTreeMap<BlockId, String>>>,
    inputs: BTreeMap<MomentId, Rc<BTreeMap<BlockId, Entity<InputState>>>>,
    diffs: BTreeMap<MomentId, Entity<DiffState>>,
    review_again: BTreeSet<MomentId>,
}

impl ViewState {
    /// Whether a row is expanded.
    pub fn is_expanded(&self, id: &MomentId) -> bool {
        self.expanded.contains(id)
    }

    /// Sets a row's disclosure; returns whether it changed.
    pub fn set_expanded(&mut self, id: &MomentId, expanded: bool) -> bool {
        if expanded { self.expanded.insert(id.clone()) } else { self.expanded.remove(id) }
    }

    /// Shows or clears the "review again" notice of a surface; returns
    /// whether it changed.
    pub fn set_review_again(&mut self, id: &MomentId, on: bool) -> bool {
        if on { self.review_again.insert(id.clone()) } else { self.review_again.remove(id) }
    }

    /// Drops every "review again" notice.
    pub fn clear_reviews(&mut self) {
        self.review_again.clear();
    }

    /// Whether a surface shows the "review again" notice.
    pub fn is_review_again(&self, id: &MomentId) -> bool {
        self.review_again.contains(id)
    }

    /// Records a select block's value; returns whether it changed.
    pub fn set_field(&mut self, id: &MomentId, block: &BlockId, value: String) -> bool {
        let mut fields = self.fields.get(id).map_or_else(BTreeMap::new, |f| (**f).clone());
        if fields.get(block) == Some(&value) {
            return false;
        }
        fields.insert(block.clone(), value);
        self.fields.insert(id.clone(), Rc::new(fields));
        true
    }

    /// The state handed to a row renderer.
    pub fn row_state(&self, id: &MomentId, highlighted: bool) -> RowState {
        RowState {
            expanded: self.expanded.contains(id),
            highlighted,
            fields: self.fields.get(id).cloned().unwrap_or_default(),
            inputs: self.inputs.get(id).cloned().unwrap_or_default(),
            diff: self.diffs.get(id).cloned(),
            focus: None,
            review_again: self.review_again.contains(id),
        }
    }

    /// Creates the retained states rows need: a text input per surface input
    /// block and a compare view per conflict with lines.
    pub fn ensure<'a>(
        &mut self,
        rows: impl Iterator<Item = &'a MomentRow>,
        window: &mut Window,
        cx: &mut App,
    ) {
        for row in rows {
            let id = &row.moment.id;
            match &row.moment.kind {
                MomentKind::AgentSurface(surface) => {
                    let wanted: Vec<&BlockId> = surface
                        .blocks
                        .iter()
                        .filter(|b| b.kind == BlockKind::Input)
                        .map(|b| &b.id)
                        .collect();
                    let current = self.inputs.get(id).cloned().unwrap_or_default();
                    if wanted.iter().all(|b| current.contains_key(*b)) {
                        continue;
                    }
                    let mut inputs = (*current).clone();
                    for block in surface.blocks.iter().filter(|b| b.kind == BlockKind::Input) {
                        let placeholder = block
                            .data
                            .get("placeholder")
                            .and_then(JsonValue::as_str)
                            .unwrap_or_default()
                            .to_owned();
                        inputs.entry(block.id.clone()).or_insert_with(|| {
                            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
                        });
                    }
                    self.inputs.insert(id.clone(), Rc::new(inputs));
                }
                MomentKind::Conflict(conflict) if !self.diffs.contains_key(id) => {
                    if conflict.local.lines.is_empty() && conflict.remote.lines.is_empty() {
                        continue;
                    }
                    let name = row
                        .refs
                        .objects
                        .get(&conflict.object_id)
                        .map_or_else(|| conflict.object_id.to_string(), |o| o.label());
                    let patch =
                        crate::moments::conflict_patch(&name, &conflict.local, &conflict.remote);
                    if let Ok(files) = DiffFile::parse(&patch) {
                        let diff =
                            cx.new(|cx| DiffState::new(files, cx).with_mode(DiffMode::Split));
                        self.diffs.insert(id.clone(), diff);
                    }
                }
                _ => {}
            }
        }
    }

    /// The values a surface's inputs and selects hold, keyed by each block's
    /// `name`, for the moment level to add to an `ActionInvoked` event (B3).
    pub fn values(&self, row: &MomentRow, cx: &App) -> JsonValue {
        let MomentKind::AgentSurface(surface) = &row.moment.kind else {
            return JsonValue::Null;
        };
        let id = &row.moment.id;
        let fields = self.fields.get(id);
        let inputs = self.inputs.get(id);
        let mut values = serde_json::Map::new();
        for block in &surface.blocks {
            let Some(name) = block.data.get("name").and_then(JsonValue::as_str) else { continue };
            let value = match block.kind {
                BlockKind::Select => fields.and_then(|f| f.get(&block.id).cloned()).or_else(|| {
                    block.data.get("value").and_then(JsonValue::as_str).map(str::to_owned)
                }),
                BlockKind::Input => inputs
                    .and_then(|i| i.get(&block.id))
                    .map(|input| input.read(cx).value().to_string()),
                _ => None,
            };
            if let Some(value) = value {
                values.insert(name.to_owned(), JsonValue::String(value));
            }
        }
        if values.is_empty() { JsonValue::Null } else { JsonValue::Object(values) }
    }
}
