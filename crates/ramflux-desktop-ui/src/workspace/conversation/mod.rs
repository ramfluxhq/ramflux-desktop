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
mod rename;
mod rows;
mod view_state;

use std::rc::Rc;

use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::message_scroller::{MessageScroller, MessageScrollerState};
use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, Render,
    StyleRefinement, Subscription, WeakEntity, Window, div,
};
use ramflux_desktop_core::{ContextId, Listing, MomentId, Scope, UiEvent, UiEventKind};

use crate::app::{ChangeSet, StoreEvent, StoreHandle};
use crate::bubble::EventSink;
use crate::i18n::tr;
use crate::moments::MomentView;
use crate::root::{ConfirmHost, IntentRoot};

use super::composer::Composer;
use super::new_conversation::NewConversation;
use rows::{RowUpdate, Rows};
use view_state::ViewState;

pub use header::{TIMERS, timer_label};

/// The conversation field view.
pub struct Conversation {
    store: Entity<StoreHandle>,
    root: Entity<IntentRoot>,
    root_sink: EventSink,
    context_id: Option<ContextId>,
    rows: Rows,
    scroller: Entity<MessageScrollerState>,
    view: ViewState,
    highlighted: Option<MomentId>,
    composer: Entity<Composer>,
    draft: Option<Entity<NewConversation>>,
    group_form: Option<Entity<crate::workspace::new_group::NewGroup>>,
    rename: Option<rename::Rename>,
    rename_subscription: Option<Subscription>,
    /// The conversation whose call waits for the stale-verification
    /// acknowledgement (view-local).
    call_ack: Option<ContextId>,
    focus_handle: FocusHandle,
    highlight_focus: FocusHandle,
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
                this.follow_rename(cx);
                if changes.is_reset() {
                    // A new world (scenario, account switch): a "review again"
                    // notice belongs to the old one.
                    this.view.clear_reviews();
                }
                // The authority refused an action: the surface changed after
                // the user reviewed it (rows below remeasure: they changed).
                for id in changes.surface_refusals() {
                    this.view.set_review_again(id, true);
                }
                let Some(context_id) = this.context_id.clone() else {
                    // The placeholder tells loading from no conversations.
                    if changes.touches_listings() {
                        cx.notify();
                    }
                    return;
                };
                if changes.touches_context(store.read(cx).store(), &context_id) {
                    this.sync_rows(Some(changes), cx);
                }
            }),
            cx.observe(&root, |_, _, cx| cx.notify()),
            // Rows keep translated text (day headers): build them again.
            crate::i18n::on_locale_switch(cx, |this, cx| this.retranslate_rows(cx)),
        ];
        Self {
            root_sink: IntentRoot::sink(&root),
            store,
            root,
            context_id: None,
            rows: Rows::default(),
            scroller,
            view: ViewState::default(),
            highlighted: None,
            composer,
            draft: None,
            group_form: None,
            rename: None,
            rename_subscription: None,
            call_ack: None,
            focus_handle: cx.focus_handle(),
            highlight_focus: cx.focus_handle().tab_stop(true),
            _subscriptions: subscriptions,
        }
    }

    /// Shows a new-conversation draft in place of a context, or stops.
    pub fn set_draft(&mut self, draft: Option<Entity<NewConversation>>, cx: &mut Context<Self>) {
        if self.draft.as_ref().map(Entity::entity_id) != draft.as_ref().map(Entity::entity_id) {
            self.draft = draft;
            cx.notify();
        }
    }

    /// Shows the new-group form in place of a context (without the
    /// composer), or stops.
    pub fn set_group_form(
        &mut self,
        form: Option<Entity<crate::workspace::new_group::NewGroup>>,
        cx: &mut Context<Self>,
    ) {
        if self.group_form.as_ref().map(Entity::entity_id) != form.as_ref().map(Entity::entity_id) {
            self.group_form = form;
            cx.notify();
        }
    }

    /// Opens the inline rename of the shown group conversation.
    pub fn start_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(context) = self.context_id.clone() else { return };
        let title = self.store.read(cx).store().context(&context).map(|c| c.title.clone());
        let input =
            cx.new(|cx| InputState::new(window, cx).default_value(title.unwrap_or_default()));
        input.update(cx, |input, cx| input.focus(window, cx));
        self.rename_subscription =
            Some(cx.subscribe_in(&input, window, |this, _, event, window, cx| match event {
                InputEvent::PressEnter { .. } => this.save_rename(window, cx),
                InputEvent::Change => {
                    if let Some(rename) = &mut this.rename
                        && matches!(rename.phase, rename::Phase::Failed(_))
                    {
                        rename.phase = rename::Phase::Editing;
                        cx.notify();
                    }
                }
                _ => {}
            }));
        self.rename = Some(rename::Rename { context, input, phase: rename::Phase::Editing });
        cx.notify();
    }

    /// Submits the inline rename.
    pub fn save_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(rename) = &mut self.rename else { return };
        let title = rename.take(self.store.read(cx).store(), cx);
        let sink = self.root_sink.scoped(Scope::Context(rename.context.clone()));
        if let Some(title) = title {
            sink.emit(UiEventKind::RenameRequested { title }, window, cx);
        }
        self.follow_rename(cx);
        cx.notify();
    }

    /// Closes the inline rename without renaming.
    pub fn cancel_rename(&mut self, cx: &mut Context<Self>) {
        if self.rename.take().is_some() {
            self.rename_subscription = None;
            cx.notify();
        }
    }

    /// Whether a rename is open, for tests.
    pub fn is_renaming(&self) -> bool {
        self.rename.is_some()
    }

    fn follow_rename(&mut self, cx: &mut Context<Self>) {
        let Some(rename) = &mut self.rename else { return };
        if rename.follow(self.store.read(cx).store()) {
            self.rename = None;
            self.rename_subscription = None;
        }
        cx.notify();
    }

    /// The shown context.
    pub fn context_id(&self) -> Option<&ContextId> {
        self.context_id.as_ref()
    }

    /// Ids of the shown rows in timeline order.
    pub fn row_ids(&self) -> Vec<MomentId> {
        self.rows.as_slice().iter().map(|row| row.moment.id.clone()).collect()
    }

    /// Number of shown rows.
    pub fn row_count(&self) -> usize {
        self.rows.as_slice().len()
    }

    /// Whether the shown row of `id` has its message text (false while the
    /// body is being loaded again).
    pub fn has_body(&self, id: &MomentId) -> bool {
        self.rows
            .position(id)
            .and_then(|ix| self.rows.as_slice().get(ix))
            .is_some_and(|row| row.body.is_some())
    }

    /// The scroller state (virtual list bookkeeping), for tests.
    pub fn scroller(&self) -> &Entity<MessageScrollerState> {
        &self.scroller
    }

    /// Day headers in row order (`None` for rows that continue a day).
    pub fn day_headers(&self) -> Vec<Option<String>> {
        self.rows
            .as_slice()
            .iter()
            .map(|row| row.day_header.as_ref().map(ToString::to_string))
            .collect()
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
        self.rename = None;
        self.rename_subscription = None;
        self.view = ViewState::default();
        self.highlighted = None;
        self.rows.clear();
        self.sync_rows(None, cx);
    }

    /// Highlights a moment and scrolls it into view.
    pub fn focus_moment(&mut self, id: &MomentId, cx: &mut Context<Self>) {
        let Some(ix) = self.rows.position(id) else { return };
        self.highlighted = Some(id.clone());
        self.scroller.update(cx, |state, cx| {
            let _ = state.scroll_to_item(ix, cx);
        });
        cx.notify();
    }

    /// Moves keyboard focus to the highlighted row, so Tab reaches its
    /// controls (approve, deny, details) next.
    pub fn focus_highlighted(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.highlighted.is_some() {
            window.focus(&self.highlight_focus, cx);
            cx.notify();
        }
    }

    /// The keyboard focus of the highlighted row.
    pub fn highlight_focus(&self) -> &FocusHandle {
        &self.highlight_focus
    }

    fn remeasure(&mut self, id: &MomentId, cx: &mut Context<Self>) {
        if let Some(ix) = self.rows.position(id) {
            self.scroller.update(cx, |state, cx| {
                let _ = state.remeasure_items(ix..ix + 1, cx);
            });
        }
        cx.notify();
    }

    /// Asks the user to review a surface again: an action on it was not
    /// performed because it changed after the user reviewed it.
    pub fn review_again(&mut self, id: &MomentId, cx: &mut Context<Self>) {
        if self.view.set_review_again(id, true) {
            self.remeasure(id, cx);
        }
    }

    /// Whether a surface row shows the "review again" notice.
    pub fn is_review_again(&self, id: &MomentId) -> bool {
        self.view.is_review_again(id)
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

    /// Brings the rows up to date (patched for `changes` when possible)
    /// and tells the virtual list exactly what moved.
    fn sync_rows(&mut self, changes: Option<&ChangeSet>, cx: &mut Context<Self>) {
        let update = match &self.context_id {
            Some(id) => self.rows.sync(self.store.read(cx), id, changes),
            None => self.rows.clear(),
        };
        let count = self.rows.as_slice().len();
        self.scroller.update(cx, |state, cx| match update {
            RowUpdate::None => {}
            RowUpdate::Reset => state.reset(count, cx),
            RowUpdate::Patched { changed, appended } => {
                for ix in changed {
                    let _ = state.remeasure_items(ix..ix + 1, cx);
                }
                if appended > 0 {
                    let _ = state.append(appended, cx);
                }
            }
        });
        cx.notify();
    }

    /// Builds every row again in the new locale, keeping the scroll
    /// position; rows whose text changed length are measured again.
    fn retranslate_rows(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.context_id.clone() else { return };
        let _ = self.rows.sync(self.store.read(cx), &id, None);
        let count = self.rows.as_slice().len();
        self.scroller.update(cx, |state, cx| {
            if state.item_count() == count {
                state.remeasure(cx);
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
                UiEventKind::ActionInvoked { action_id, values, surface_hash, version } => {
                    let (action_id, surface_hash, version) =
                        (action_id.clone(), surface_hash.clone(), *version);
                    // Acting again is the new review: the notice goes away.
                    let collected = this
                        .update(cx, |field, cx| {
                            if field.view.set_review_again(&moment_id, false) {
                                field.remeasure(&moment_id, cx);
                            }
                            let row = field.rows.position(&moment_id)?;
                            field.rows.as_slice().get(row).map(|row| field.view.values(row, cx))
                        })
                        .ok()
                        .flatten()
                        .unwrap_or_default();
                    let values = if values.is_null() { collected } else { values.clone() };
                    Some(UiEvent {
                        scope_path: event.scope_path,
                        kind: UiEventKind::ActionInvoked {
                            action_id,
                            values,
                            surface_hash,
                            version,
                        },
                    })
                }
                _ => Some(event),
            },
        )
    }
}

impl Render for Conversation {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.rows.shared();
        self.view.ensure(self.rows.stateful(), window, cx);
        let store = self.store.read(cx).store();
        let context = self.context_id.as_ref().and_then(|id| store.context(id));
        let context_sink =
            self.context_id.clone().map(|id| self.root_sink.scoped(Scope::Context(id)));
        let this = cx.weak_entity();
        let renaming = header::Renaming {
            field: self
                .rename
                .as_ref()
                .filter(|r| Some(&r.context) == self.context_id.as_ref())
                .map(|rename| {
                    rename::render(
                        rename,
                        cx.listener(|this, _, window, cx| this.save_rename(window, cx)),
                        cx.listener(|this, _, _, cx| this.cancel_rename(cx)),
                        cx,
                    )
                }),
            start: Rc::new(move |window, cx| {
                this.update(cx, |field, cx| field.start_rename(window, cx)).ok();
            }),
        };
        let (ask, close) = (cx.weak_entity(), cx.weak_entity());
        let call_ack = header::CallAck {
            open: self.call_ack.is_some() && self.call_ack == self.context_id,
            ask: Rc::new(move |_, cx| {
                ask.update(cx, |field, cx| {
                    field.call_ack.clone_from(&field.context_id);
                    cx.notify();
                })
                .ok();
            }),
            close: Rc::new(move |_, cx| {
                close
                    .update(cx, |field, cx| {
                        field.call_ack = None;
                        cx.notify();
                    })
                    .ok();
            }),
        };
        let ask_call = call_ack.ask.clone();
        let header = header::render(store, context, context_sink.clone(), renaming, call_ack, cx);
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
                // Only visible rows render: the per-row view state is looked
                // up in the renderer, not prepared for every row.
                let view = Rc::new(self.view.clone());
                let highlighted = self.highlighted.clone();
                let highlight_focus = self.highlight_focus.clone();
                let this = cx.weak_entity();
                let store = self.store.downgrade();
                let ask_call = ask_call.clone();
                MessageScroller::new("conversation", self.scroller.clone(), move |ix, _, cx| {
                    let Some(row) = rows.get(ix) else {
                        return div().into_any_element();
                    };
                    let id = &row.moment.id;
                    if row.is_missing_body() {
                        let (store, id) = (store.clone(), id.clone());
                        cx.defer(move |cx| {
                            store.update(cx, |store, _| store.request_bodies(&[id])).ok();
                        });
                    }
                    let is_highlighted = highlighted.as_ref() == Some(id);
                    let mut state = view.row_state(id, is_highlighted);
                    state.focus = is_highlighted.then(|| highlight_focus.clone());
                    state.ask_call = Some(ask_call.clone());
                    let sink = Self::moment_sink(this.clone(), &context_sink, id);
                    MomentView::new(row.clone(), sink, state).into_any_element()
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
        let draft = self.draft.clone().filter(|_| self.context_id.is_none());
        let group_form = self.group_form.clone().filter(|_| self.context_id.is_none());
        let composer = group_form.is_none().then(|| self.composer.clone());
        v_flex()
            .id("conversation")
            .track_focus(&self.focus_handle)
            .size_full()
            .min_w_0()
            .bg(cx.theme().background)
            .map(|field| match (group_form, draft) {
                (Some(form), _) => field.child(form),
                (None, Some(draft)) => field.child(draft),
                (None, None) => field.child(header).child(body),
            })
            .children(strip)
            .children(composer)
    }
}
