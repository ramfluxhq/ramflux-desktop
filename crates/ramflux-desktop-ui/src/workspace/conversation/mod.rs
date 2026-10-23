// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The conversation field: the current context's timeline, the confirmation
//! strip and the composer. It is the center panel of the workspace dock.
//!
//! Rows are grouped by local day (today, yesterday, date headers). The field
//! owns view-local state only ([`view_state::ViewState`]): expanded rows,
//! surface form fields, conflict compare views, the highlighted row. The
//! moment level consumes expand and field edits (rule B1) and adds collected
//! form values to surface actions before they bubble on (rule B3).

mod header;
mod view_state;

use std::rc::Rc;

use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent};
use gpui_kit::component::message_scroller::{MessageScroller, MessageScrollerState};
use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, Render,
    StyleRefinement, Subscription, WeakEntity, Window, div,
};
use ramflux_desktop_core::{ContextId, Listing, MomentId, Scope, UiEvent, UiEventKind, selectors};

use crate::app::{StoreEvent, StoreHandle};
use crate::bubble::EventSink;
use crate::i18n::{self, tr};
use crate::moments::{MomentRow, MomentView};
use crate::root::{ConfirmHost, IntentRoot};

use super::composer::Composer;
use view_state::ViewState;

pub use header::{TIMERS, timer_label};

/// The conversation field view.
pub struct Conversation {
    store: Entity<StoreHandle>,
    root: Entity<IntentRoot>,
    root_sink: EventSink,
    context_id: Option<ContextId>,
    rows: Rc<Vec<MomentRow>>,
    scroller: Entity<MessageScrollerState>,
    view: ViewState,
    highlighted: Option<MomentId>,
    composer: Entity<Composer>,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<PanelEvent> for Conversation {}

impl Focusable for Conversation {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BasePanel for Conversation {
    fn panel_name(&self) -> &'static str {
        // Must match the name the workspace persists layouts under.
        "Conversation"
    }

    fn closable(&self, _: &App) -> bool {
        false
    }

    fn zoomable(&self, _: &App) -> bool {
        false
    }
}

impl Panel for Conversation {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        tr("conversation.title")
    }

    fn inner_padding(&self, _: &App) -> bool {
        false
    }

    fn title_bar(&self, _: &App) -> bool {
        false
    }
}

impl Conversation {
    /// A field with no context, emitting through the root sink.
    pub fn new(
        store: Entity<StoreHandle>,
        root: Entity<IntentRoot>,
        composer: Entity<Composer>,
        cx: &mut Context<Self>,
    ) -> Self {
        let scroller = cx.new(|cx| MessageScrollerState::new(0, cx));
        let subscriptions = vec![
            cx.subscribe(&store, |this, store, event: &StoreEvent, cx| {
                let StoreEvent::Changed(changes) = event else { return };
                let Some(context_id) = this.context_id.clone() else {
                    // The placeholder tells loading from no conversations.
                    if changes.touches_listings() {
                        cx.notify();
                    }
                    return;
                };
                if changes.touches_context(store.read(cx).store(), &context_id) {
                    this.sync_rows(cx);
                }
            }),
            cx.observe(&root, |_, _, cx| cx.notify()),
        ];
        Self {
            root_sink: IntentRoot::sink(&root),
            store,
            root,
            context_id: None,
            rows: Rc::new(Vec::new()),
            scroller,
            view: ViewState::default(),
            highlighted: None,
            composer,
            focus_handle: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// The shown context.
    pub fn context_id(&self) -> Option<&ContextId> {
        self.context_id.as_ref()
    }

    /// Ids of the shown rows in timeline order.
    pub fn row_ids(&self) -> Vec<MomentId> {
        self.rows.iter().map(|row| row.moment.id.clone()).collect()
    }

    /// Day headers in row order (`None` for rows that continue a day).
    pub fn day_headers(&self) -> Vec<Option<String>> {
        self.rows.iter().map(|row| row.day_header.as_ref().map(ToString::to_string)).collect()
    }

    /// Whether a row's disclosure is expanded.
    pub fn is_expanded(&self, id: &MomentId) -> bool {
        self.view.is_expanded(id)
    }

    /// The highlighted row.
    pub fn highlighted(&self) -> Option<&MomentId> {
        self.highlighted.as_ref()
    }

    /// Shows another context. View-local state of the previous one is dropped.
    pub fn set_context(&mut self, context_id: Option<ContextId>, cx: &mut Context<Self>) {
        if self.context_id == context_id {
            return;
        }
        self.context_id = context_id;
        self.view = ViewState::default();
        self.highlighted = None;
        self.rows = Rc::new(Vec::new());
        self.sync_rows(cx);
    }

    /// Highlights a moment and scrolls it into view.
    pub fn focus_moment(&mut self, id: &MomentId, cx: &mut Context<Self>) {
        let Some(ix) = self.rows.iter().position(|row| &row.moment.id == id) else { return };
        self.highlighted = Some(id.clone());
        self.scroller.update(cx, |state, cx| {
            let _ = state.scroll_to_item(ix, cx);
        });
        cx.notify();
    }

    fn remeasure(&mut self, id: &MomentId, cx: &mut Context<Self>) {
        if let Some(ix) = self.rows.iter().position(|row| &row.moment.id == id) {
            self.scroller.update(cx, |state, cx| {
                let _ = state.remeasure_items(ix..ix + 1, cx);
            });
        }
        cx.notify();
    }

    /// Toggles a row's disclosure. Called from the moment-level sink.
    pub fn set_expanded(&mut self, id: &MomentId, expanded: bool, cx: &mut Context<Self>) {
        if self.view.set_expanded(id, expanded) {
            self.remeasure(id, cx);
        }
    }

    /// The pin command for the shown context.
    pub fn toggle_pin(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(context_id) = self.context_id.clone() else { return };
        let pinned = self.store.read(cx).store().context(&context_id).is_some_and(|c| c.pinned);
        self.root_sink.scoped(Scope::Context(context_id)).emit(
            UiEventKind::PinToggled { pinned: !pinned },
            window,
            cx,
        );
    }

    fn sync_rows(&mut self, cx: &mut Context<Self>) {
        let rows: Vec<MomentRow> = match &self.context_id {
            Some(id) => {
                let handle = self.store.read(cx);
                let store = handle.store();
                let now = handle.session().map(|s| s.now()).unwrap_or_default();
                let me = selectors::self_participant(store);
                let mut last_day = None;
                selectors::timeline(store, id)
                    .into_iter()
                    .map(|m| {
                        let mut row = MomentRow::build(store, m, me.as_ref());
                        let day = i18n::clock::local_day(m.at);
                        if day != last_day {
                            row.day_header = Some(i18n::day_label(m.at, now));
                            last_day = day;
                        }
                        row
                    })
                    .collect()
            }
            None => Vec::new(),
        };
        let old = Rc::clone(&self.rows);
        let same_prefix = old.len() <= rows.len()
            && old.iter().zip(rows.iter()).all(|(a, b)| a.moment.id == b.moment.id);
        let changed: Vec<usize> = old
            .iter()
            .zip(rows.iter())
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .map(|(ix, _)| ix)
            .collect();
        let added = rows.len().saturating_sub(old.len());
        let count = rows.len();
        self.rows = Rc::new(rows);
        self.scroller.update(cx, |state, cx| {
            if same_prefix && state.item_count() == old.len() {
                for ix in changed {
                    let _ = state.remeasure_items(ix..ix + 1, cx);
                }
                if added > 0 {
                    let _ = state.append(added, cx);
                }
            } else {
                state.reset(count, cx);
            }
        });
        cx.notify();
    }

    /// The moment-level sink: consumes expand and field edits (B1) and adds
    /// the surface's form values to action invocations (B3).
    fn moment_sink(this: WeakEntity<Self>, context_sink: &EventSink, id: &MomentId) -> EventSink {
        let moment_id = id.clone();
        context_sink.scoped_map(
            Scope::Moment(id.clone()),
            move |event: UiEvent, _, cx| match &event.kind {
                UiEventKind::ExpandToggled { expanded } => {
                    let expanded = *expanded;
                    this.update(cx, |field, cx| field.set_expanded(&moment_id, expanded, cx)).ok();
                    None
                }
                UiEventKind::FocusMomentRequested { moment_id: target } => {
                    let target = target.clone();
                    this.update(cx, |field, cx| {
                        field.set_expanded(&target, true, cx);
                        field.focus_moment(&target, cx);
                    })
                    .ok();
                    None
                }
                UiEventKind::FieldChanged { value } => {
                    let block = event.block_id()?.clone();
                    let value = value.clone();
                    this.update(cx, |field, cx| {
                        if field.view.set_field(&moment_id, &block, value) {
                            field.remeasure(&moment_id, cx);
                        }
                    })
                    .ok();
                    None
                }
                UiEventKind::ActionInvoked { action_id, values } if values.is_null() => {
                    let action_id = action_id.clone();
                    let collected = this
                        .read_with(cx, |field, cx| {
                            field
                                .rows
                                .iter()
                                .find(|row| row.moment.id == moment_id)
                                .map(|row| field.view.values(row, cx))
                        })
                        .ok()
                        .flatten()
                        .unwrap_or_default();
                    Some(UiEvent {
                        scope_path: event.scope_path,
                        kind: UiEventKind::ActionInvoked { action_id, values: collected },
                    })
                }
                _ => Some(event),
            },
        )
    }
}

impl Render for Conversation {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = Rc::clone(&self.rows);
        self.view.ensure(&rows, window, cx);
        let store = self.store.read(cx).store();
        let context = self.context_id.as_ref().and_then(|id| store.context(id));
        let context_sink =
            self.context_id.clone().map(|id| self.root_sink.scoped(Scope::Context(id)));
        let header = header::render(store, context, context_sink.clone(), cx);
        let body = match context_sink.filter(|_| !rows.is_empty()) {
            None => {
                let no_contexts = store.contexts().next().is_none();
                let state = crate::list_state::of(store, Listing::Contexts, no_contexts);
                let placeholder = if self.context_id.is_some() {
                    None
                } else {
                    crate::list_state::placeholder(
                        "conversation-list-state",
                        state,
                        "conversation.no_contexts",
                        cx,
                    )
                };
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .map(|area| match placeholder {
                        Some(placeholder) => area.child(placeholder),
                        None => area.child(tr(if self.context_id.is_some() {
                            "conversation.empty"
                        } else {
                            "conversation.choose"
                        })),
                    })
                    .into_any_element()
            }
            Some(context_sink) => {
                let states: Rc<Vec<_>> = Rc::new(
                    rows.iter()
                        .map(|row| {
                            let id = &row.moment.id;
                            self.view.row_state(id, self.highlighted.as_ref() == Some(id))
                        })
                        .collect(),
                );
                let this = cx.weak_entity();
                MessageScroller::new("conversation", self.scroller.clone(), move |ix, _, _| {
                    let (Some(row), Some(state)) = (rows.get(ix), states.get(ix)) else {
                        return div().into_any_element();
                    };
                    let sink = Self::moment_sink(this.clone(), &context_sink, &row.moment.id);
                    MomentView::new(row.clone(), sink, state.clone()).into_any_element()
                })
                .with_jump_button_label(tr("conversation.jump_latest"))
                .with_row_style(StyleRefinement::default().pb_2())
                .flex_1()
                .min_h_0()
                .into_any_element()
            }
        };
        let root = self.root.read(cx);
        let strip = (root.host() == ConfirmHost::Workspace).then(|| root.confirm_strip().clone());
        v_flex()
            .id("conversation")
            .track_focus(&self.focus_handle)
            .size_full()
            .min_w_0()
            .bg(cx.theme().background)
            .child(header)
            .child(body)
            .children(strip)
            .child(self.composer.clone())
    }
}
