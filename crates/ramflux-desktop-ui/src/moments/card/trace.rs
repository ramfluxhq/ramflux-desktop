// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Draw trace of the card gallery (test builds only, feature `card-gallery`).
//!
//! While a card body is drawn, every node the renderer walks is recorded
//! under the card's surface id. Conformance check B reads the trace after the
//! frame to see which nodes were drawn, whether any was a placeholder, and
//! whether every host slot's node was drawn. Nothing here runs in other
//! builds.

use std::cell::RefCell;
use std::collections::BTreeMap;

/// The twenty kinds the runtime defines (catalog order).
pub const KINDS: [&str; 20] = [
    "Text",
    "Icon",
    "Row",
    "Column",
    "List",
    "Card",
    "Tabs",
    "Modal",
    "Divider",
    "Button",
    "TextField",
    "CheckBox",
    "ChoicePicker",
    "Slider",
    "DateTimeInput",
    "Image",
    "Approval",
    "Task",
    "Chart",
    "Table",
];

/// One node the renderer walked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Drawn {
    /// The node key.
    pub key: String,
    /// The node kind as the tree names it.
    pub kind: String,
    /// Whether the kind is one the renderer draws (false: a placeholder).
    pub known: bool,
}

thread_local! {
    static CURRENT: RefCell<Option<String>> = const { RefCell::new(None) };
    static DRAWN: RefCell<BTreeMap<String, Vec<Drawn>>> = const { RefCell::new(BTreeMap::new()) };
}

/// Starts recording the nodes drawn for `surface`.
pub fn begin(surface: &str) {
    CURRENT.with(|current| *current.borrow_mut() = Some(surface.to_owned()));
}

/// Stops recording.
pub fn end() {
    CURRENT.with(|current| *current.borrow_mut() = None);
}

/// Records for the lifetime of the guard; stops recording when dropped.
pub struct Scope;

impl Scope {
    /// Starts recording the nodes drawn for `surface`.
    #[must_use]
    pub fn begin(surface: &str) -> Self {
        begin(surface);
        Self
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        end();
    }
}

/// Records one node the renderer walked, under the surface being drawn.
pub fn drawn(key: &str, kind: &str) {
    let Some(surface) = CURRENT.with(|current| current.borrow().clone()) else {
        return;
    };
    let entry = Drawn { key: key.to_owned(), kind: kind.to_owned(), known: KINDS.contains(&kind) };
    DRAWN.with(|rows| rows.borrow_mut().entry(surface).or_default().push(entry));
}

/// Takes what was drawn for `surface` since it was last taken.
pub fn take(surface: &str) -> Vec<Drawn> {
    DRAWN.with(|rows| rows.borrow_mut().remove(surface).unwrap_or_default())
}

/// Forgets everything recorded so far.
pub fn clear() {
    DRAWN.with(|rows| rows.borrow_mut().clear());
}
