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
mod selection;
mod view_state;

use ramflux_desktop_navigation::direction::DirectionalExt as _;
use ramflux_desktop_navigation::scale::{space, text_secondary};
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
    rename: Option<rename::Rename>,
    rename_subscription: Option<Subscription>,
    focus_handle: FocusHandle,
    highlight_focus: FocusHandle,
    /// The dock tool open beside the conversation, if any.
    open_tool: Option<crate::workspace::DockTool>,
    /// The window's status banner, shown under the header (design B38)
    /// while this view is the workspace's detail.
    status: Option<gpui_kit::AnyView>,
    /// The oldest row shown when older history was last asked for: asked
    /// again only once older rows arrived (and were scrolled to).
    older_asked: Option<MomentId>,
    /// The wait until a used-up daily limit of an MLS group resets: it only
    /// redraws, never sends. Dropping it cancels it.
    rate_timer: Option<(ramflux_desktop_core::Timestamp, gpui_kit::Task<()>)>,
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
    /// Notes the dock tool open beside the conversation: the header shows
    /// the tool switcher only while none is (the open panel carries it).
    pub fn set_open_tool(&mut self, tool: Option<crate::workspace::DockTool>) {
        self.open_tool = tool;
    }

    /// Hands the window's status banner to this view (or takes it back).
    pub fn set_status(&mut self, status: Option<gpui_kit::AnyView>) {
        self.status = status;
    }

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
            older_asked: None,
            rate_timer: None,
            view: ViewState::default(),
            highlighted: None,
            composer,
            draft: None,
            rename: None,
            rename_subscription: None,
            focus_handle: cx.focus_handle(),
            highlight_focus: cx.focus_handle().tab_stop(true),
            open_tool: None,
            status: None,
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

    /// Native text-selection state, absent after eviction, deletion or lock.
    pub fn message_text_state(
        &self,
        id: &MomentId,
    ) -> Option<&Entity<gpui_kit::component::input::TextareaState>> {
        self.view.message_text(id)
    }

    /// Retained message-selection entries, created entities, and rows examined
    /// by the latest source delta; counters expose the virtualization contract.
    pub fn message_selection_counts(&self) -> (usize, usize, usize) {
        (self.view.selection.len(), self.view.selection.created, self.view.selection.last_touched)
    }

    /// The wait for a used-up daily limit ended: the composer and Retry come
    /// back (nothing is sent).
    fn rate_lock_elapsed(&mut self, cx: &mut Context<Self>) {
        self.rate_timer = None;
        if let Some(id) = self.context_id.clone() {
            let _ = self.rows.sync(self.store.read(cx), &id, None);
        }
        cx.notify();
    }

    /// Keeps one timer for the lock of the shown group: armed for its reset
    /// time, dropped when there is no lock (an account lock clears it).
    fn follow_rate_lock(
        &mut self,
        until: Option<ramflux_desktop_core::Timestamp>,
        now: ramflux_desktop_core::Timestamp,
        cx: &mut Context<Self>,
    ) {
        match until {
            None => self.rate_timer = None,
            Some(until) if self.rate_timer.as_ref().map(|(at, _)| *at) != Some(until) => {
                let wait = std::time::Duration::from_millis(until.saturating_since(now) + 50);
                let task = cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(wait).await;
                    this.update(cx, |field, cx| field.rate_lock_elapsed(cx)).ok();
                });
                self.rate_timer = Some((until, task));
            }
            Some(_) => {}
        }
    }

    /// The oldest row shown came into view: the source is asked for the
    /// history before it, once per oldest row.
    fn reached_top(&mut self, top: &MomentId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(context) = self.context_id.clone() else { return };
        if self.older_asked.as_ref() == Some(top) {
            return;
        }
        // An MLS group asks for the history before its oldest post through
        // the intent (once per oldest row, and only while older rows exist).
        if let Some(group) = ramflux_desktop_core::mls_group::group_of(&context) {
            let oldest = self.store.read(cx).store().mls_timeline(group).and_then(|tl| {
                (tl.more_older && !tl.older_in_flight)
                    .then(|| tl.posted.keys().next().copied())
                    .flatten()
            });
            if let Some(oldest) = oldest {
                self.older_asked = Some(top.clone());
                self.root_sink.scoped(Scope::Context(context.clone())).emit(
                    UiEventKind::MlsHistoryRequested {
                        group_id: group.to_owned(),
                        direction: ramflux_desktop_core::mls_group::HistoryDirection::Before(
                            oldest,
                        ),
                    },
                    window,
                    cx,
                );
            }
            return;
        }
        self.older_asked = Some(top.clone());
        self.store.update(cx, |store, _| store.request_older(&context));
    }

    fn ensure_visible_message(
        &mut self,
        id: &MomentId,
        visible: &std::collections::BTreeSet<MomentId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let created = self
            .rows
            .position(id)
            .and_then(|ix| self.rows.as_slice().get(ix))
            .is_some_and(|row| self.view.selection.ensure(row, visible, window, cx));
        if created {
            if let Some(input) = self.view.selection.text(id).cloned() {
                let id = id.clone();
                let observed_id = id.clone();
                let subscription =
                    cx.observe(&input, move |this, _, cx| this.remeasure(&observed_id, cx));
                self.view.selection.observe(&id, subscription);
            }
            self.remeasure(id, cx);
        }
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
        let before: Vec<MomentId> =
            self.rows.as_slice().iter().map(|row| row.moment.id.clone()).collect();
        let update = match &self.context_id {
            Some(id) => self.rows.sync(self.store.read(cx), id, changes),
            None => self.rows.clear(),
        };
        // A rebuild that kept every row in order only re-measures, and one
        // that put older rows before them (a page of older history) prepends
        // them: the reader stays where they were.
        let kept = matches!(update, RowUpdate::Reset)
            .then(|| {
                let after = self.rows.as_slice();
                let added = after.len().checked_sub(before.len())?;
                (!before.is_empty()
                    && after[added..].iter().map(|row| &row.moment.id).eq(before.iter()))
                .then_some(added)
            })
            .flatten();
        self.view.selection.last_touched = 0;
        match &update {
            RowUpdate::Reset => self.view.selection.clear(),
            RowUpdate::Patched { changed, appended } => {
                for ix in changed.iter().copied().chain(
                    self.rows.as_slice().len().saturating_sub(*appended)
                        ..self.rows.as_slice().len(),
                ) {
                    if let Some(row) = self.rows.as_slice().get(ix) {
                        self.view.selection.changed(row);
                    }
                }
            }
            RowUpdate::None => {}
        }
        let count = self.rows.as_slice().len();
        self.scroller.update(cx, |state, cx| match update {
            RowUpdate::None => {}
            RowUpdate::Reset if kept == Some(0) => state.remeasure(cx),
            RowUpdate::Reset if let Some(added) = kept => {
                let _ = state.prepend(added, cx);
                state.remeasure(cx);
            }
            RowUpdate::Reset => state.reset(count, cx),
            RowUpdate::Patched { changed, appended } => {
                for ix in changed {
                    let end = (ix + 2).min(count);
                    let _ = state.remeasure_items(ix..end, cx);
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
            move |event: UiEvent, window, cx| match &event.kind {
                UiEventKind::CopyMessagePressed => {
                    this.update(cx, |field, cx| {
                        let store = field.store.read(cx).store();
                        if ramflux_desktop_core::projection::conversations::is_visible(store)
                            && let Some(moment) = store.moment(&moment_id)
                            && Some(&moment.context_id) == field.context_id.as_ref()
                            && matches!(&moment.kind, ramflux_desktop_core::MomentKind::Message(p) if !p.tombstone)
                            && let Some(text) = store.body(&moment_id) {
                            cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(text.to_owned()));
                        }
                    }).ok();
                    None
                }
                // Deleting an own post of an MLS group asks first: the
                // confirmation dialog sends the delete.
                UiEventKind::DeleteForEveryonePressed
                    if moment_id.as_str().starts_with(ramflux_desktop_core::mls_group::CONTEXT_PREFIX) =>
                {
                    let (group, post) = this
                        .update(cx, |field, cx| {
                            let store = field.store.read(cx).store();
                            let moment = store.moment(&moment_id)?;
                            let group = ramflux_desktop_core::mls_group::group_of(&moment.context_id)?
                                .to_owned();
                            let post = crate::root::intent_map::mls_post_of(store, &moment_id)?;
                            Some((group, post))
                        })
                        .ok()
                        .flatten()?;
                    Some(UiEvent {
                        scope_path: event.scope_path.clone(),
                        kind: UiEventKind::MlsPanelRequested {
                            panel: ramflux_desktop_core::mls_group::MlsPanel::ConfirmDelete {
                                group_id: group,
                                post_id: post,
                            },
                        },
                    })
                }
                // Reply and Edit open in the composer; what it submits is
                // the action (SubmitReply, SubmitEdit), scoped to the
                // conversation.
                UiEventKind::ReplyPressed | UiEventKind::EditPressed => {
                    let edit = matches!(event.kind, UiEventKind::EditPressed);
                    this.update(cx, |field, cx| {
                        let store = field.store.read(cx).store();
                        let Some(moment) = store.moment(&moment_id) else { return };
                        if Some(&moment.context_id) != field.context_id.as_ref() {
                            return;
                        }
                        let text = store.body(&moment_id).unwrap_or_default().to_owned();
                        let author: gpui_kit::SharedString = store
                            .participant(&moment.origin)
                            .map_or_else(|| moment.origin.short(), |p| p.display.clone())
                            .into();
                        let excerpt = ramflux_desktop_navigation::row::list_text(&text);
                        let target = moment_id.clone();
                        field.composer.clone().update(cx, |composer, cx| {
                            if edit {
                                composer.edit(target, text, window, cx);
                            } else {
                                composer.reply_to(target, author, excerpt, window, cx);
                            }
                        });
                    })
                    .ok();
                    None
                }
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
                UiEventKind::ActionInvoked { action_id, values, surface_hash, version } => {
                    let (action_id, surface_hash, version) =
                        (action_id.clone(), surface_hash.clone(), *version);
                    let values = values.clone();
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
        // The cards of the loaded window: any other session is released.
        let loaded_cards: std::collections::BTreeSet<ramflux_desktop_core::SurfaceId> = rows
            .iter()
            .filter_map(|row| row.card.as_ref().map(|card| card.surface_id.clone()))
            .collect();
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
        let in_form = self.context_id.is_none() && self.draft.is_some();
        let open_tool = self.open_tool;
        let switcher = || {
            crate::workspace::DockRail::new(open_tool, |tool, window, cx| {
                window.dispatch_action(tool.toggle_action(), cx)
            })
            .into_any_element()
        };
        // Board H01: the header keeps its tools while a panel is open.
        let tools = (!in_form).then(switcher);
        // A new conversation or group form has no header: the switcher sits
        // in a row of its own above it.
        let tools_row = (self.open_tool.is_none() && in_form).then(|| {
            ramflux_desktop_navigation::direction::h_flex()
                .justify_row_end()
                .px(gpui_kit::rems(32.0 / 15.0))
                .pt(space(3.))
                .child(switcher())
        });
        let mls_group = context
            .filter(|c| c.kind == ramflux_desktop_core::ContextKind::MlsGroup)
            .and_then(|c| ramflux_desktop_core::mls_group::group_of(&c.id))
            .map(str::to_owned);
        let header = match (context.filter(|_| mls_group.is_some()), context_sink.clone()) {
            // An MLS group has its own header: no rename, no old group menu.
            (Some(context), Some(sink)) => {
                super::mls_group::chrome::header(store, context, sink, tools, cx)
            }
            _ => header::render(
                store,
                context,
                context_sink.clone(),
                renaming,
                header::Controls { tools },
                cx,
            ),
        };
        // A read-only MLS group has a banner and no composer (S13); an open
        // one has the strip of the members the next post mentions.
        let mls_banner = mls_group
            .as_deref()
            .and_then(|g| super::mls_group::chrome::read_only_banner(store, g, cx));
        let mls_strip = mls_group
            .as_deref()
            .zip(context_sink.clone())
            .and_then(|(g, sink)| super::mls_group::chrome::mention_strip(store, g, sink, cx));
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
                    .text_size(text_secondary())
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
                let visible = Rc::new(std::cell::RefCell::new(std::collections::BTreeSet::new()));
                MessageScroller::new(
                    "conversation",
                    self.scroller.clone(),
                    move |ix, window, cx| {
                        let Some(row) = rows.get(ix) else {
                            return div().into_any_element();
                        };
                        let id = &row.moment.id;
                        if ix == 0 {
                            let (this, id) = (this.clone(), id.clone());
                            window.defer(cx, move |window, cx| {
                                this.update(cx, |field, cx| field.reached_top(&id, window, cx))
                                    .ok();
                            });
                        }
                        if row.is_missing_body() {
                            let (store, id) = (store.clone(), id.clone());
                            cx.defer(move |cx| {
                                store.update(cx, |store, _| store.request_bodies(&[id])).ok();
                            });
                        }
                        if let Some(card) = row.card.as_ref() {
                            // A card on screen is opened (or presented in the app's
                            // locale and theme) once; the store is asked, not drawn.
                            let (store, moment, surface) =
                                (store.clone(), id.clone(), card.surface_id.clone());
                            let (locale, dark) = (crate::i18n::locale(), cx.theme().is_dark());
                            cx.defer(move |cx| {
                                store
                                    .update(cx, |store, cx| {
                                        store.card_on_screen(&moment, &surface, &locale, dark, cx)
                                    })
                                    .ok();
                            });
                        }
                        if ix == 0 {
                            let (store, loaded) = (store.clone(), loaded_cards.clone());
                            cx.defer(move |cx| {
                                store.update(cx, |store, cx| store.card_retain(&loaded, cx)).ok();
                            });
                        }
                        if matches!(row.moment.kind, ramflux_desktop_core::MomentKind::Message(_)) {
                            visible.borrow_mut().insert(id.clone());
                            let (this, id, visible) = (this.clone(), id.clone(), visible.clone());
                            window.defer(cx, move |window, cx| {
                                this.update(cx, |field, cx| {
                                    field.ensure_visible_message(&id, &visible.borrow(), window, cx)
                                })
                                .ok();
                            });
                        }
                        let is_highlighted = highlighted.as_ref() == Some(id);
                        let mut state = view.row_state(id, is_highlighted);
                        state.joined =
                            ix.checked_sub(1).and_then(|at| rows.get(at)).is_some_and(|previous| {
                                crate::moments::message_bubble::joins(previous, row)
                            });
                        state.focus = is_highlighted.then(|| highlight_focus.clone());
                        let sink = Self::moment_sink(this.clone(), &context_sink, id);
                        // Design board 02: rows sit in a centred 880 px
                        // reading column.
                        let column = ramflux_desktop_navigation::palette(cx).column_max;
                        div()
                            .w_full()
                            .flex()
                            .justify_center()
                            .child(
                                div()
                                    .w_full()
                                    .max_w(gpui_kit::rems(column / 15.0))
                                    .child(MomentView::new(row.clone(), sink, state)),
                            )
                            .into_any_element()
                    },
                )
                .with_jump_button_label(tr("conversation.jump_latest"))
                .with_row_style(StyleRefinement::default().pb(space(3.)))
                .flex_1()
                .min_h_0()
                .into_any_element()
            }
        };
        let root = self.root.read(cx);
        let strip = (root.host() == ConfirmHost::Workspace).then(|| root.confirm_strip().clone());
        let draft = self.draft.clone().filter(|_| self.context_id.is_none());
        // A used-up daily limit takes the composer away until it resets.
        let now = self.store.read(cx).session().map(|s| s.now()).unwrap_or_default();
        let rate_until = mls_group
            .as_deref()
            .and_then(|g| self.store.read(cx).store().mls_rate_locked_until(g, now));
        self.follow_rate_lock(rate_until, now, cx);
        let rate_tip = rate_until.filter(|_| mls_banner.is_none()).map(|until| {
            div()
                .id("mls-rate-limit")
                .role(gpui_kit::Role::Status)
                .w_full()
                .px(gpui_kit::rems(24.0 / 15.0))
                .py(space(3.))
                .text_size(text_secondary())
                .text_color(cx.theme().muted_foreground)
                .child(crate::i18n::tr_args(
                    "group.send.rate_limited",
                    &[("time", &crate::i18n::time_of_day(until))],
                ))
        });
        let mls_strip = mls_strip.filter(|_| rate_tip.is_none());
        let composer = (mls_banner.is_none() && rate_tip.is_none()).then(|| self.composer.clone());
        let status = self.status.clone();
        v_flex()
            .id("conversation")
            .track_focus(&self.focus_handle)
            .size_full()
            .min_w_0()
            .bg(cx.theme().background)
            .map(|field| match draft {
                Some(draft) => field.children(tools_row).child(draft),
                None => field.child(header).children(status).child(body),
            })
            .children(strip)
            .children(mls_banner)
            .children(mls_strip)
            .children(rate_tip)
            // Board H01: the composer keeps 24 px gutters when the pane is
            // narrower than its reading column (a tool panel open).
            .children(
                composer
                    .map(|composer| div().w_full().px(gpui_kit::rems(24.0 / 15.0)).child(composer)),
            )
    }
}
