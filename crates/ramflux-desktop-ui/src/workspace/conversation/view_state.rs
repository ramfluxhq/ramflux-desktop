// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! View-local state of the conversation field's rows (rule R4): which rows
//! are expanded, card text field drafts and card presentation, conflict
//! compare states. Nothing here reaches the store; it is dropped when the
//! field shows another context.

use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use gpui_kit::component::diff::{DiffFile, DiffMode, DiffState};
use gpui_kit::component::input::InputState;
use gpui_kit::{App, AppContext as _, Entity, Window};
use ramflux_desktop_core::{CardNode, JsonValue, MomentId, MomentKind};

use crate::moments::card::{Inputs, Local};
use crate::moments::{MomentRow, RowState};

/// One text field's sync: the node key, the held value and whether a
/// refusal or a discarded draft forces the reset.
struct FieldSync<'a> {
    key: &'a str,
    tree_value: &'a str,
    forced: bool,
}

/// Brings a text field's draft to the held value of its node. The draft
/// follows the tree only while it shows what the desktop last sent or synced
/// for the field: typing the user has not sent is kept. A forced sync resets
/// the draft regardless.
fn sync_field(
    entity: &Entity<InputState>,
    sync: FieldSync<'_>,
    local: &crate::moments::card::Local,
    window: &mut Window,
    cx: &mut App,
) {
    let FieldSync { key, tree_value, forced } = sync;
    let typed = entity.read(cx).value().to_string();
    let mut state = local.borrow_mut();
    let changed = state.synced.get(key).map(String::as_str) != Some(tree_value);
    let shown = state.sent.get(key).or_else(|| state.synced.get(key)).cloned();
    let untouched = shown.as_deref().is_none_or(|shown| shown == typed);
    if forced || (changed && untouched) {
        if typed != tree_value {
            entity.update(cx, |input, cx| input.set_value(tree_value.to_owned(), window, cx));
        }
        state.sent.remove(key);
        state.synced.insert(key.to_owned(), tree_value.to_owned());
    }
}

/// Per-row view-local state. Cloning is cheap (shared maps of handles),
/// so the row renderer gets a snapshot per frame.
#[derive(Clone, Default)]
pub struct ViewState {
    expanded: BTreeSet<MomentId>,
    pub(super) selection: super::selection::Cache,
    /// Text field drafts of each card, by node key.
    inputs: BTreeMap<MomentId, Inputs>,
    /// Tabs, modal and list presentation of each card.
    locals: BTreeMap<MomentId, Local>,
    diffs: BTreeMap<MomentId, Entity<DiffState>>,
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

    /// The state handed to a row renderer.
    pub fn row_state(&self, id: &MomentId, highlighted: bool) -> RowState {
        RowState {
            expanded: self.expanded.contains(id),
            joined: false,
            menu_focus: self.selection.focus(id),
            message_text: self.selection.text(id).cloned(),
            highlighted,
            card_inputs: self.inputs.get(id).cloned().unwrap_or_default(),
            card_local: self.locals.get(id).cloned().unwrap_or_default(),
            diff: self.diffs.get(id).cloned(),
            focus: None,
        }
    }

    /// Native selection participant while a live, cached plain body exists.
    pub fn message_text(
        &self,
        id: &MomentId,
    ) -> Option<&Entity<gpui_kit::component::input::TextareaState>> {
        self.selection.text(id)
    }

    /// Creates the retained states rows need: a text input per card text
    /// field (holding the runtime's value at first), the card's local
    /// presentation, and a compare view per conflict with lines.
    pub fn ensure<'a>(
        &mut self,
        rows: impl Iterator<Item = &'a MomentRow>,
        window: &mut Window,
        cx: &mut App,
    ) {
        for row in rows {
            let id = &row.moment.id;
            match &row.moment.kind {
                MomentKind::AgentSurface(_) => {
                    let local = self.locals.entry(id.clone()).or_default().clone();
                    let record = row.card.as_ref().and_then(|c| c.record.as_ref());
                    let fields: Vec<&CardNode> = record
                        .and_then(|r| r.tree.as_ref())
                        .map(|tree| {
                            tree.nodes()
                                .filter(|n| n.kind == "TextField" || n.kind == "DateTimeInput")
                                .collect()
                        })
                        .unwrap_or_default();
                    let current = self.inputs.get(id).cloned().unwrap_or_default();
                    // A refused change or a discarded draft resets every field
                    // to the held value, once per tree version.
                    let tree_version =
                        record.and_then(|r| r.tree.as_ref()).map(|t| t.tree_version());
                    let forced = record.is_some_and(|r| r.refusal.is_some() || r.draft_discarded)
                        && tree_version.is_some()
                        && local.borrow().reset_at != tree_version;
                    // Keep the entity of each field still shown; drop the rest.
                    let mut inputs = BTreeMap::new();
                    for node in fields {
                        let tree_value = node
                            .attrs
                            .get("value")
                            .and_then(JsonValue::as_str)
                            .unwrap_or_default()
                            .to_owned();
                        let entity = match current.get(&node.key).cloned() {
                            Some(entity) => entity,
                            None => {
                                let value = tree_value.clone();
                                let entity =
                                    cx.new(|cx| InputState::new(window, cx).default_value(value));
                                let mut state = local.borrow_mut();
                                state.sent.remove(&node.key);
                                state.synced.insert(node.key.clone(), tree_value.clone());
                                entity
                            }
                        };
                        sync_field(
                            &entity,
                            FieldSync { key: &node.key, tree_value: &tree_value, forced },
                            &local,
                            window,
                            cx,
                        );
                        inputs.insert(node.key.clone(), entity);
                    }
                    if forced {
                        local.borrow_mut().reset_at = tree_version;
                    }
                    if inputs.len() != current.len()
                        || inputs.keys().any(|k| !current.contains_key(k))
                    {
                        self.inputs.insert(id.clone(), Rc::new(inputs));
                    }
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
}
