// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The conversation field: the current context's timeline, the confirmation
//! strip and the composer. It is the center panel of the workspace dock.
//!
//! The field owns view-local state only: which rows are expanded and which
//! row is highlighted. Expand clicks are consumed at the moment level and
//! never reach the root (rule B1).

use std::collections::BTreeSet;
use std::rc::Rc;

use gpui_kit::component::StyledExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent};
use gpui_kit::component::message_scroller::{MessageScroller, MessageScrollerState};
use gpui_kit::component::{ActiveTheme as _, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, Render,
    StyleRefinement, Subscription, WeakEntity, Window, div,
};
use ramflux_desktop_core::{ContextId, MomentId, Scope, Store, UiEventKind, selectors};

use crate::app::{StoreEvent, StoreHandle};
use crate::bubble::EventSink;
use crate::confirm::ConfirmStrip;
use crate::i18n::{tr, tr_args};
use crate::moments::{MomentRow, MomentView};

use super::composer::Composer;

/// The conversation field view.
pub struct Conversation {
    store: Entity<StoreHandle>,
    root_sink: EventSink,
    context_id: Option<ContextId>,
    rows: Rc<Vec<MomentRow>>,
    scroller: Entity<MessageScrollerState>,
    expanded: Rc<BTreeSet<MomentId>>,
    highlighted: Option<MomentId>,
    confirm: Entity<ConfirmStrip>,
    composer: Entity<Composer>,
    focus_handle: FocusHandle,
    _subscription: Subscription,
}

impl EventEmitter<PanelEvent> for Conversation {}

impl Focusable for Conversation {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BasePanel for Conversation {
    fn panel_name(&self) -> &'static str {
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
    /// A field with no context, emitting through the workspace `root_sink`.
    pub fn new(
        store: Entity<StoreHandle>,
        root_sink: EventSink,
        confirm: Entity<ConfirmStrip>,
        composer: Entity<Composer>,
        cx: &mut Context<Self>,
    ) -> Self {
        let scroller = cx.new(|cx| MessageScrollerState::new(0, cx));
        let subscription = cx.subscribe(&store, |this, store, event: &StoreEvent, cx| {
            let StoreEvent::Changed(changes) = event else { return };
            let Some(context_id) = this.context_id.clone() else { return };
            if changes.touches_context(store.read(cx).store(), &context_id) {
                this.sync_rows(cx);
            }
        });
        Self {
            store,
            root_sink,
            context_id: None,
            rows: Rc::new(Vec::new()),
            scroller,
            expanded: Rc::new(BTreeSet::new()),
            highlighted: None,
            confirm,
            composer,
            focus_handle: cx.focus_handle(),
            _subscription: subscription,
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

    /// Whether a row's disclosure is expanded.
    pub fn is_expanded(&self, id: &MomentId) -> bool {
        self.expanded.contains(id)
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
        self.expanded = Rc::new(BTreeSet::new());
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

    /// Toggles a row's disclosure. Called from the moment-level sink.
    pub fn set_expanded(&mut self, id: &MomentId, expanded: bool, cx: &mut Context<Self>) {
        let mut set = (*self.expanded).clone();
        let changed = if expanded { set.insert(id.clone()) } else { set.remove(id) };
        if !changed {
            return;
        }
        self.expanded = Rc::new(set);
        if let Some(ix) = self.rows.iter().position(|row| &row.moment.id == id) {
            self.scroller.update(cx, |state, cx| {
                let _ = state.remeasure_items(ix..ix + 1, cx);
            });
        }
        cx.notify();
    }

    /// Emits a pin toggle for the shown context (the pin command).
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
                let store = self.store.read(cx).store();
                selectors::timeline(store, id)
                    .into_iter()
                    .map(|m| MomentRow::build(store, m))
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

    fn moment_sink(this: WeakEntity<Self>, context_sink: &EventSink, id: &MomentId) -> EventSink {
        let moment_id = id.clone();
        context_sink.scoped_with(Scope::Moment(id.clone()), move |event, _, cx| {
            let UiEventKind::ExpandToggled { expanded } = event.kind else { return false };
            this.update(cx, |field, cx| field.set_expanded(&moment_id, expanded, cx)).ok();
            true
        })
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let store: &Store = self.store.read(cx).store();
        let context = self.context_id.as_ref().and_then(|id| store.context(id));
        let title = context.map_or_else(|| tr("conversation.none"), |c| c.title.clone().into());
        let members = context.map_or(0, |c| c.participants.len());
        let pinned = context.is_some_and(|c| c.pinned);
        let has_context = context.is_some();
        h_flex()
            .id("conversation-header")
            .w_full()
            .gap_2()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(div().font_semibold().text_color(cx.theme().foreground).child(title))
            .when(members > 0, |header| {
                header.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(tr_args("conversation.members", &[("count", &members.to_string())])),
                )
            })
            .child(div().flex_1())
            .when(has_context, |header| {
                header.child(
                    Button::new("conversation-pin")
                        .ghost()
                        .small()
                        .icon(IconName::Star)
                        .label(tr(if pinned { "conversation.unpin" } else { "conversation.pin" }))
                        .on_click(cx.listener(|this, _, window, cx| this.toggle_pin(window, cx))),
                )
            })
    }
}

impl Render for Conversation {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let header = self.render_header(cx);
        let body = match self.context_id.clone().filter(|_| !self.rows.is_empty()) {
            None => div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(tr(if self.context_id.is_some() {
                    "conversation.empty"
                } else {
                    "conversation.choose"
                }))
                .into_any_element(),
            Some(context_id) => {
                let rows = Rc::clone(&self.rows);
                let expanded = Rc::clone(&self.expanded);
                let highlighted = self.highlighted.clone();
                let context_sink = self.root_sink.scoped(Scope::Context(context_id));
                let this = cx.weak_entity();
                MessageScroller::new("conversation", self.scroller.clone(), move |ix, _, _| {
                    let Some(row) = rows.get(ix) else {
                        return div().into_any_element();
                    };
                    let id = row.moment.id.clone();
                    let sink = Self::moment_sink(this.clone(), &context_sink, &id);
                    MomentView::new(row.clone(), sink)
                        .expanded(expanded.contains(&id))
                        .highlighted(highlighted.as_ref() == Some(&id))
                        .into_any_element()
                })
                .with_jump_button_label(tr("conversation.jump_latest"))
                .with_row_style(StyleRefinement::default().pb_2())
                .flex_1()
                .min_h_0()
                .into_any_element()
            }
        };
        v_flex()
            .id("conversation")
            .track_focus(&self.focus_handle)
            .size_full()
            .min_w_0()
            .bg(cx.theme().background)
            .child(header)
            .child(body)
            .child(self.confirm.clone())
            .child(self.composer.clone())
    }
}
