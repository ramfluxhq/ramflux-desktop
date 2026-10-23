// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The conversation workspace (P1) and the root of UI event bubbling.
//!
//! Layout: context stack | dock area (conversation field in the center,
//! summoned tools on the right) | dock rail. The terminal status strip lives
//! in the window title bar (see `shell`).
//!
//! The workspace root is the only place UI events become intents: it maps
//! them with [`crate::bubble::intent_for`], hands the intent to the
//! [`StoreHandle`], and opens the hold-to-confirm strip when policy asks for
//! human confirmation (rule B4). It keeps the pending intent itself, so the
//! only way to submit a confirmed intent is a completed hold.

mod composer;
mod context_stack;
mod conversation;
mod docks;
mod status_strip;

use std::sync::Arc;
use std::time::Instant;

use gpui_kit::component::dock::{
    BasePanelView, DockArea, DockLayout, DockPlacement, DockSkin, DockToggleEvent, panel_handle,
};
use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, Entity, FocusHandle, Focusable, IntoElement, Render, Subscription, Window, px,
};
use ramflux_desktop_core::{
    Actor, ContextId, Intent, IntentKind, ParticipantId, Scope, UiEvent, selectors,
};

use crate::app::{
    DispatchOutcome, InspectorEntry, StoreEvent, StoreHandle, UiOutcome, format_path, format_scope,
    intent_name, ui_event_name,
};
use crate::bubble::{EventSink, intent_for};
use crate::confirm::{ConfirmEvent, ConfirmRequest, ConfirmStrip};

pub use composer::Composer;
pub use context_stack::{ContextStack, ContextStackEvent};
pub use conversation::Conversation;
pub use docks::{DockRail, DockTool, DockToolPanel};
pub use status_strip::StatusStrip;

/// Width of the right dock when it first opens. A physical layout seed for
/// the dock engine, which sizes docks in pixels; users resize it freely.
const DOCK_WIDTH: f32 = 320.;

/// The workspace view.
pub struct Workspace {
    store: Entity<StoreHandle>,
    sink: EventSink,
    stack: Entity<ContextStack>,
    conversation: Entity<Conversation>,
    composer: Entity<Composer>,
    confirm: Entity<ConfirmStrip>,
    dock_area: Entity<DockArea>,
    tools: Vec<(DockTool, Arc<dyn BasePanelView>)>,
    open_tool: Option<DockTool>,
    current: Option<ContextId>,
    pending: Option<Intent>,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for Workspace {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Workspace {
    /// A workspace over `store`.
    pub fn new(store: Entity<StoreHandle>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let sink = root_sink(cx);
        let confirm = cx.new(ConfirmStrip::new);
        let composer = cx.new(|cx| Composer::new(window, cx));
        let stack = cx.new(|cx| ContextStack::new(store.clone(), cx));
        let conversation = cx.new(|cx| {
            Conversation::new(store.clone(), sink.clone(), confirm.clone(), composer.clone(), cx)
        });

        let (dock_area, _skin) = DockSkin::dock_area("workspace-dock", Some(1), window, cx);
        let tools: Vec<(DockTool, Arc<dyn BasePanelView>)> = DockTool::ALL
            .into_iter()
            .map(|tool| (tool, panel_handle(cx.new(|cx| DockToolPanel::new(tool, cx)))))
            .collect();
        dock_area.update(cx, |area, cx| {
            let center = DockLayout::tabs().panel_view(panel_handle(conversation.clone()), cx);
            area.set_center(center, window, cx);
            let right = tools.iter().fold(DockLayout::tabs(), |layout, (_, panel)| {
                layout.panel_view(panel.clone(), cx)
            });
            area.set_dock(DockPlacement::Right, right, window, cx);
            area.set_dock_collapsible(DockPlacement::Right, true, window, cx);
            area.set_dock_size(DockPlacement::Right, px(DOCK_WIDTH), window, cx);
            if area.is_dock_open(DockPlacement::Right) {
                area.toggle_dock(DockPlacement::Right, window, cx);
            }
        });

        let subscriptions = vec![
            cx.subscribe_in(&store, window, |this, _, event: &StoreEvent, window, cx| {
                if let StoreEvent::Changed(changes) = event
                    && changes.touches_context_stack()
                {
                    this.ensure_current(window, cx);
                }
            }),
            cx.subscribe_in(&stack, window, |this, _, event: &ContextStackEvent, window, cx| {
                let ContextStackEvent::Selected(id) = event;
                this.select_context(Some(id.clone()), window, cx);
            }),
            cx.subscribe_in(&confirm, window, |this, _, event: &ConfirmEvent, window, cx| {
                this.on_confirm_event(*event, window, cx);
            }),
            cx.subscribe(&dock_area, |this, _, event: &DockToggleEvent, cx| {
                if !event.is_open() {
                    this.open_tool = None;
                    cx.notify();
                }
            }),
        ];

        let mut workspace = Self {
            store,
            sink,
            stack,
            conversation,
            composer,
            confirm,
            dock_area,
            tools,
            open_tool: None,
            current: None,
            pending: None,
            focus_handle: cx.focus_handle(),
            _subscriptions: subscriptions,
        };
        workspace.ensure_current(window, cx);
        workspace
    }

    /// The shown context.
    pub fn current_context(&self) -> Option<&ContextId> {
        self.current.as_ref()
    }

    /// The conversation field.
    pub fn conversation(&self) -> &Entity<Conversation> {
        &self.conversation
    }

    /// The composer.
    pub fn composer(&self) -> &Entity<Composer> {
        &self.composer
    }

    /// The hold-to-confirm strip.
    pub fn confirm_strip(&self) -> &Entity<ConfirmStrip> {
        &self.confirm
    }

    /// The intent waiting for human confirmation.
    pub fn pending_intent(&self) -> Option<&Intent> {
        self.pending.as_ref()
    }

    /// The open dock tool.
    pub fn open_tool(&self) -> Option<DockTool> {
        self.open_tool
    }

    /// Shows a context, or nothing.
    pub fn select_context(
        &mut self,
        id: Option<ContextId>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.current = id.clone();
        let sink = id.as_ref().map(|id| self.sink.scoped(Scope::Context(id.clone())));
        self.conversation.update(cx, |field, cx| field.set_context(id.clone(), cx));
        self.composer.update(cx, |composer, cx| composer.set_sink(sink, cx));
        self.stack.update(cx, |stack, cx| stack.set_current(id, cx));
        cx.notify();
    }

    /// Keeps the current context valid: when it disappears (or none is
    /// chosen yet) the first stacked context, else the first context, is shown.
    fn ensure_current(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let store = self.store.read(cx).store();
        if self.current.as_ref().is_some_and(|id| store.context(id).is_some()) {
            return;
        }
        let next = selectors::context_stack(store)
            .into_iter()
            .next()
            .or_else(|| store.contexts().next().map(|c| c.id.clone()));
        if next != self.current {
            self.select_context(next, window, cx);
        }
    }

    /// "Take me to what needs me": shows the oldest attention item.
    pub fn go_to_needs_me(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = selectors::attention(self.store.read(cx).store()).into_iter().next()
        else {
            return;
        };
        if self.current.as_ref() != Some(&item.context_id) {
            self.select_context(Some(item.context_id.clone()), window, cx);
        }
        self.conversation.update(cx, |field, cx| field.focus_moment(&item.moment_id, cx));
    }

    /// The pin command for the current context.
    ///
    /// Takes the entity rather than `&mut self`: the field emits through the
    /// root sink, which updates this workspace, so the workspace must not be
    /// borrowed while the field runs.
    pub fn toggle_pin(workspace: &Entity<Self>, window: &mut Window, cx: &mut App) {
        let field = workspace.read(cx).conversation.clone();
        field.update(cx, |field, cx| field.toggle_pin(window, cx));
    }

    /// Opens a dock tool, or closes it when it is already open.
    pub fn toggle_tool(&mut self, tool: DockTool, window: &mut Window, cx: &mut Context<Self>) {
        let close = self.open_tool == Some(tool);
        let panel = self.tools.iter().find(|(t, _)| *t == tool).map(|(_, p)| p.panel_id(cx));
        self.dock_area.update(cx, |area, cx| {
            let open = area.is_dock_open(DockPlacement::Right);
            if close == open {
                area.toggle_dock(DockPlacement::Right, window, cx);
            }
            if let (false, Some(panel)) = (close, panel) {
                area.select_panel(panel, window, cx);
            }
        });
        self.open_tool = if close { None } else { Some(tool) };
        cx.notify();
    }

    /// Submits an intent proposed by an agent participant. It follows the
    /// same policy as human intents and can never carry confirmation (R6).
    pub fn submit_agent_intent(
        &mut self,
        agent: ParticipantId,
        kind: IntentKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.submit(Intent { actor: Actor::Agent(agent), kind }, window, cx);
    }

    fn on_ui_event(&mut self, event: UiEvent, window: &mut Window, cx: &mut Context<Self>) {
        let started = Instant::now();
        let path = format_path(&event.scope_path);
        let kind = ui_event_name(&event);
        let intent = intent_for(&event, self.store.read(cx).store()).map(Intent::human);
        let outcome = match intent {
            None => UiOutcome::Ignored,
            Some(intent) => {
                let name = intent_name(&intent).to_owned();
                let result = self.submit(intent, window, cx);
                UiOutcome::Intent { intent: name, result }
            }
        };
        let entry = InspectorEntry::UiEvent { path, kind, outcome, elapsed: started.elapsed() };
        self.store.update(cx, |store, cx| store.record(entry, cx));
    }

    fn record_consumed(&mut self, event: &UiEvent, scope: &Scope, cx: &mut Context<Self>) {
        let entry = InspectorEntry::UiEvent {
            path: format_path(&event.scope_path),
            kind: ui_event_name(event),
            outcome: UiOutcome::Consumed { at: format_scope(scope) },
            elapsed: std::time::Duration::ZERO,
        };
        self.store.update(cx, |store, cx| store.record(entry, cx));
    }

    /// Runs policy and either submits, opens the confirmation strip, or
    /// reports the refusal. Returns a short result for the inspector.
    fn submit(&mut self, intent: Intent, window: &mut Window, cx: &mut Context<Self>) -> String {
        let request = ConfirmRequest::describe(self.store.read(cx).store(), &intent);
        match self.store.update(cx, |store, cx| store.dispatch(intent, cx)) {
            DispatchOutcome::Submitted(labels) => format!("submitted {}", labels.join(", ")),
            DispatchOutcome::NeedsConfirmation(intent) => {
                self.pending = Some(*intent);
                self.confirm.update(cx, |strip, cx| strip.open(request, window, cx));
                cx.notify();
                "confirmation required".to_owned()
            }
            DispatchOutcome::Refused(error) => format!("refused: {error}"),
            DispatchOutcome::NoAccount => "no account".to_owned(),
        }
    }

    fn on_confirm_event(
        &mut self,
        event: ConfirmEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(intent) = self.pending.take() else { return };
        match event {
            ConfirmEvent::HoldCompleted => {
                self.submit(intent.into_human_confirmed(), window, cx);
            }
            ConfirmEvent::Cancelled => {
                let entry = InspectorEntry::Intent {
                    intent: intent_name(&intent),
                    actor: format!("{:?}", intent.actor),
                    result: "confirmation cancelled".to_owned(),
                };
                self.store.update(cx, |store, cx| store.record(entry, cx));
            }
        }
        self.composer.update(cx, |composer, cx| composer.focus(window, cx));
        cx.notify();
    }
}

fn root_sink(cx: &mut Context<Workspace>) -> EventSink {
    let handler = cx.weak_entity();
    let observer = handler.clone();
    EventSink::root(
        move |event, window, cx| {
            handler.update(cx, |workspace, cx| workspace.on_ui_event(event, window, cx)).ok();
        },
        move |event, scope, cx| {
            observer.update(cx, |workspace, cx| workspace.record_consumed(event, scope, cx)).ok();
        },
    )
}

impl Render for Workspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.weak_entity();
        h_flex()
            .id("workspace")
            .track_focus(&self.focus_handle)
            .items_stretch()
            .size_full()
            .min_h_0()
            .bg(cx.theme().background)
            .child(self.stack.clone())
            .child(gpui_kit::div().flex_1().min_w_0().h_full().child(self.dock_area.clone()))
            .child(DockRail::new(self.open_tool, move |tool, window, cx| {
                this.update(cx, |workspace, cx| workspace.toggle_tool(tool, window, cx)).ok();
            }))
    }
}
