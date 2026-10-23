// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Restoring the persisted dock area.
//!
//! The dock engine rebuilds panels by name through its global registry. The
//! workspace already owns every panel (they are wired to the store and the
//! intent root), so the builders registered here hand back those existing
//! panels instead of creating new ones; the registration is made right before
//! the load it serves. A saved tree is used only when it was written under
//! [`LAYOUT_SCHEMA`] and names exactly the panels this build has, each once;
//! anything else falls back to the default layout.

use std::collections::BTreeMap;
use std::sync::Arc;

use gpui_kit::App;
use gpui_kit::component::dock::{
    BasePanelView, DockAreaState, PanelInfo, PanelState, register_panel,
};

use super::prefs::LAYOUT_SCHEMA;

/// Dock engine version stamped on dumps, so a dump carries the schema.
pub fn engine_version() -> Option<usize> {
    usize::try_from(LAYOUT_SCHEMA).ok()
}

/// Registers builders that return the workspace's own panels by name.
pub fn register_existing(panels: &[(&'static str, Arc<dyn BasePanelView>)], cx: &mut App) {
    for (name, handle) in panels {
        let handle = handle.clone();
        register_panel(cx, name, move |_, _, _| handle.clone());
    }
}

/// Whether `state` can be loaded: written under the current schema and
/// naming exactly `expected`, each panel once.
pub fn restorable(state: &DockAreaState, expected: &[&str]) -> bool {
    if state.version != engine_version() {
        return false;
    }
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    let docks = [&state.left_dock, &state.right_dock, &state.bottom_dock];
    let trees =
        std::iter::once(&state.center).chain(docks.into_iter().flatten().map(|d| d.panel()));
    for tree in trees {
        leaves(tree, &mut seen);
    }
    seen.len() == expected.len() && expected.iter().all(|name| seen.get(*name) == Some(&1))
}

fn leaves(state: &PanelState, seen: &mut BTreeMap<String, usize>) {
    if matches!(state.info, PanelInfo::Panel(_)) && state.children.is_empty() {
        *seen.entry(state.panel_name.clone()).or_default() += 1;
    }
    for child in &state.children {
        leaves(child, seen);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(name: &str) -> PanelState {
        PanelState {
            panel_name: name.to_owned(),
            children: Vec::new(),
            info: PanelInfo::Panel(serde_json::Value::Null),
        }
    }

    fn tabs(children: Vec<PanelState>) -> PanelState {
        PanelState { panel_name: "TabPanel".to_owned(), children, info: PanelInfo::tabs(0) }
    }

    #[test]
    fn only_current_schema_trees_with_exactly_our_panels_restore() {
        let state = DockAreaState {
            version: engine_version(),
            center: tabs(vec![leaf("Conversation"), leaf("ObjectsPanel")]),
            ..Default::default()
        };
        assert!(restorable(&state, &["Conversation", "ObjectsPanel"]));
        assert!(!restorable(&state, &["Conversation"]), "an unknown panel");
        assert!(!restorable(&state, &["Conversation", "ObjectsPanel", "EvidencePanel"]));
        let old = DockAreaState { version: Some(1), ..state.clone() };
        assert!(!restorable(&old, &["Conversation", "ObjectsPanel"]), "another schema");
        let twice = DockAreaState {
            center: tabs(vec![leaf("Conversation"), leaf("Conversation")]),
            ..state
        };
        assert!(!restorable(&twice, &["Conversation"]));
    }
}
