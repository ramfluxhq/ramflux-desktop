// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Bounded, lazy native selection and message-menu focus state for visible rows.
use crate::moments::MomentRow;
use gpui_kit::component::input::TextareaState;
use gpui_kit::{App, AppContext as _, Entity, FocusHandle, SharedString, Window};
use ramflux_desktop_core::{MomentId, MomentKind};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub(super) const CAPACITY: usize = 64;
#[derive(Clone)]
struct Entry {
    observation: Option<std::rc::Rc<gpui_kit::Subscription>>,
    body: Option<SharedString>,
    text: Option<Entity<TextareaState>>,
    focus: FocusHandle,
}
#[derive(Clone, Default)]
pub(super) struct Cache {
    entries: BTreeMap<MomentId, Entry>,
    recent: VecDeque<MomentId>,
    pub created: usize,
    pub last_touched: usize,
}
impl Cache {
    pub fn observe(&mut self, id: &MomentId, subscription: gpui_kit::Subscription) {
        if let Some(entry) = self.entries.get_mut(id) {
            entry.observation = Some(std::rc::Rc::new(subscription));
        }
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn text(&self, id: &MomentId) -> Option<&Entity<TextareaState>> {
        self.entries.get(id).and_then(|entry| entry.text.as_ref())
    }
    pub fn focus(&self, id: &MomentId) -> Option<FocusHandle> {
        self.entries.get(id).map(|entry| entry.focus.clone())
    }
    pub fn clear(&mut self) {
        self.entries.clear();
        self.recent.clear();
        self.last_touched = 0;
    }
    /// Called only for rows affected by a source delta. Existing selection is
    /// retained across receipt changes; changed/erased bodies are dropped now.
    pub fn changed(&mut self, row: &MomentRow) {
        self.last_touched += 1;
        if let Some(entry) = self.entries.get(&row.moment.id)
            && (entry.body.as_ref() != plain_body(row)
                || !matches!(row.moment.kind, MomentKind::Message(_)))
        {
            self.entries.remove(&row.moment.id);
            self.recent.retain(|id| id != &row.moment.id);
        }
    }
    /// Deferred from visible-row rendering; never constructs state in render.
    pub fn ensure(
        &mut self,
        row: &MomentRow,
        visible: &BTreeSet<MomentId>,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        if !matches!(row.moment.kind, MomentKind::Message(_)) {
            return false;
        }
        let id = &row.moment.id;
        self.recent.retain(|old| old != id);
        self.recent.push_back(id.clone());
        if self.entries.contains_key(id) {
            return false;
        }
        // Every row in this layout pass is protected, including overscan. A
        // tall viewport can legitimately need more than the warm-cache floor.
        while self.entries.len() >= CAPACITY.max(visible.len()) {
            let Some(at) = self.recent.iter().position(|id| !visible.contains(id)) else {
                break;
            };
            if let Some(old) = self.recent.remove(at) {
                self.entries.remove(&old);
            }
        }
        let body = plain_body(row).cloned();
        let text = body.as_ref().map(|body| {
            cx.new(|cx| {
                TextareaState::new(window, cx).auto_grow(1, 4096).default_value(body.clone())
            })
        });
        self.created += usize::from(text.is_some());
        self.entries.insert(
            id.clone(),
            Entry { observation: None, body, text, focus: cx.focus_handle().tab_stop(true) },
        );
        true
    }
}
fn plain_body(row: &MomentRow) -> Option<&SharedString> {
    matches!(&row.moment.kind, MomentKind::Message(p) if !p.tombstone && p.spans.is_empty())
        .then_some(row.body.as_ref())
        .flatten()
}
