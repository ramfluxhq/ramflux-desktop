// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The conversation workspace (P1).
//!
//! Layout: context stack | dock area (conversation field in the center,
//! summoned tools on the right) | dock rail. The terminal status strip lives
//! in the window title bar (see `shell`), the command palette above
//! everything (see `palette`).
//!
//! Every view here emits through the [`IntentRoot`] sink; the workspace owns
//! navigation only: the shown context, which tool is open or pinned, and the
//! "take me to what needs me" cursor across contexts and accounts.

mod composer;
mod context_stack;
mod conversation;
pub mod docks;
mod status_strip;

use std::sync::Arc;

use gpui_kit::component::dock::{
    BasePanelView, DockArea, DockEvent, DockLayout, DockPlacement, DockSkin, DockToggleEvent,
    panel_handle,
};
use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, Entity, FocusHandle, Focusable, IntoElement, Render, Subscription, Window, px,
};
use ramflux_desktop_core::{ContextId, IntentKind, ParticipantId, Scope, methods, selectors};

use crate::app::{AccountAttention, StoreEvent, StoreHandle};
use crate::bubble::EventSink;
use crate::confirm::ConfirmStrip;
use crate::root::{IntentRoot, RootEvent};

pub use composer::{Composer, ComposerMode};
pub use context_stack::{ContextStack, ContextStackEvent};
pub use conversation::{Conversation, TIMERS, timer_label};
pub use docks::prefs::{DockPrefs, LAYOUT_SCHEMA, PrefsStore};
pub use docks::{DockRail, DockTool, DockToolEvent, DockToolPanel};
pub use status_strip::StatusStrip;

use docks::approvals::ApprovalsDock;
use docks::contacts::ContactsDock;
use docks::evidence::EvidenceDock;
use docks::members::MembersDock;
use docks::objects::ObjectsDock;

/// Dock engine panel name of the conversation field.
const CONVERSATION_PANEL: &str = "Conversation";

/// One dock tool: its frame entity and the handle the dock engine holds.
struct ToolSlot {
    tool: DockTool,
    panel: Entity<DockToolPanel>,
    handle: Arc<dyn BasePanelView>,
}

/// The workspace view.
pub struct Workspace {
    store: Entity<StoreHandle>,
    root: Entity<IntentRoot>,
    stack: Entity<ContextStack>,
    conversation: Entity<Conversation>,
    composer: Entity<Composer>,
    dock_area: Entity<DockArea>,
    tools: Vec<ToolSlot>,
    objects: Entity<ObjectsDock>,
    contacts: Entity<ContactsDock>,
    members: Entity<MembersDock>,
    evidence: Entity<EvidenceDock>,
    open_tool: Option<DockTool>,
    prefs: DockPrefs,
    prefs_store: PrefsStore,
    current: Option<ContextId>,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for Workspace {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Workspace {
    /// A workspace over `store`, emitting through `root`, with dock layout
    /// preferences from `prefs_store`.
    pub fn new(
        store: Entity<StoreHandle>,
        root: Entity<IntentRoot>,
        prefs_store: PrefsStore,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let sink = IntentRoot::sink(&root);
        let composer = cx.new(|cx| Composer::new(window, cx));
        let stack = cx.new(|cx| ContextStack::new(store.clone(), cx));
        let conversation =
            cx.new(|cx| Conversation::new(store.clone(), root.clone(), composer.clone(), cx));
        let objects = cx.new(|cx| {
            ObjectsDock::new(store.clone(), dock_sink(&sink, DockTool::Objects), window, cx)
        });
        let contacts = cx.new(|cx| {
            ContactsDock::new(store.clone(), dock_sink(&sink, DockTool::Contacts), window, cx)
        });
        let members = cx.new(|cx| MembersDock::new(store.clone(), sink.clone(), cx));
        let approvals = cx.new(|cx| ApprovalsDock::new(store.clone(), sink.clone(), cx));
        let evidence = cx.new(|cx| EvidenceDock::new(store.clone(), cx));
        let contents: [(DockTool, gpui_kit::AnyView); 5] = [
            (DockTool::Objects, objects.clone().into()),
            (DockTool::Contacts, contacts.clone().into()),
            (DockTool::Members, members.clone().into()),
            (DockTool::Approvals, approvals.into()),
            (DockTool::Evidence, evidence.clone().into()),
        ];
        let tools: Vec<ToolSlot> = contents
            .into_iter()
            .map(|(tool, content)| {
                let panel = cx.new(|cx| DockToolPanel::new(tool, content, cx));
                ToolSlot { tool, handle: panel_handle(panel.clone()), panel }
            })
            .collect();
        let prefs = prefs_store.load();
        let conversation_handle = panel_handle(conversation.clone());
        let (dock_area, _skin) =
            DockSkin::dock_area("workspace-dock", docks::layout::engine_version(), window, cx);
        dock_area.update(cx, |area, cx| {
            let center = DockLayout::tabs().panel_view(conversation_handle.clone(), cx);
            area.set_center(center, window, cx);
            let right = tools.iter().fold(DockLayout::tabs(), |layout, slot| {
                layout.panel_view(slot.handle.clone(), cx)
            });
            area.set_dock(DockPlacement::Right, right, window, cx);
            area.set_dock_collapsible(DockPlacement::Right, true, window, cx);
            area.set_dock_size(DockPlacement::Right, px(prefs.width), window, cx);
        });
        let mut named: Vec<(&'static str, Arc<dyn BasePanelView>)> =
            vec![(CONVERSATION_PANEL, conversation_handle)];
        named.extend(tools.iter().map(|slot| (slot.tool.panel_name(), slot.handle.clone())));
        let names: Vec<&str> = named.iter().map(|(name, _)| *name).collect();
        if let Some(layout) =
            prefs.layout.clone().filter(|layout| docks::layout::restorable(layout, &names))
        {
            docks::layout::register_existing(&named, cx);
            dock_area.update(cx, |area, cx| {
                if area.load(layout, window, cx).is_ok() {
                    area.set_dock_collapsible(DockPlacement::Right, true, window, cx);
                }
            });
        }
        // Summoned tools start closed; a pinned tool reopens below.
        dock_area.update(cx, |area, cx| {
            if area.is_dock_open(DockPlacement::Right) {
                area.toggle_dock(DockPlacement::Right, window, cx);
            }
        });

        let mut subscriptions = vec![
            cx.subscribe_in(&store, window, |this, _, event: &StoreEvent, window, cx| {
                if let StoreEvent::Changed(changes) = event {
                    if changes.touches_context_stack() {
                        this.ensure_current(window, cx);
                    }
                    if changes.touches_status() {
                        this.sync_composer(cx);
                    }
                }
            }),
            cx.subscribe_in(&stack, window, |this, _, event: &ContextStackEvent, window, cx| {
                let ContextStackEvent::Selected(id) = event;
                this.select_context(Some(id.clone()), window, cx);
            }),
            cx.subscribe_in(&root, window, |this, _, event: &RootEvent, window, cx| match event {
                RootEvent::ConfirmClosed => {
                    this.composer.update(cx, |composer, cx| composer.focus(window, cx));
                }
                RootEvent::SurfaceChanged(moment) => {
                    this.conversation.update(cx, |field, cx| field.review_again(moment, cx));
                }
            }),
            cx.subscribe(&dock_area, |this, _, event: &DockToggleEvent, cx| {
                if !event.is_open() {
                    this.open_tool = None;
                    cx.notify();
                }
            }),
            cx.subscribe(&dock_area, |this, area, event: &DockEvent, cx| {
                if matches!(event, DockEvent::LayoutChanged) {
                    let area = area.read(cx);
                    if let Some(width) = area.dock_size(DockPlacement::Right) {
                        this.prefs.width = f32::from(width);
                    }
                    let layout = Some(area.dump(cx));
                    if layout != this.prefs.layout {
                        this.prefs.layout = layout;
                        this.prefs_store.save(&this.prefs);
                    }
                }
            }),
        ];
        for slot in &tools {
            subscriptions.push(cx.subscribe_in(
                &slot.panel,
                window,
                |this, _, event: &DockToolEvent, window, cx| match *event {
                    DockToolEvent::PinToggled(tool) => this.toggle_pinned(tool, cx),
                    DockToolEvent::CloseRequested(_) => {
                        this.close_dock(window, cx);
                        this.composer.update(cx, |composer, cx| composer.focus(window, cx));
                    }
                },
            ));
        }

        let mut workspace = Self {
            store,
            root,
            stack,
            conversation,
            composer,
            dock_area,
            tools,
            objects,
            contacts,
            members,
            evidence,
            open_tool: None,
            prefs,
            prefs_store,
            current: None,
            focus_handle: cx.focus_handle(),
            _subscriptions: subscriptions,
        };
        workspace.ensure_current(window, cx);
        if let Some(tool) = workspace.prefs.pinned {
            workspace.sync_pins(cx);
            workspace.open_tool(tool, window, cx);
        }
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

    /// The objects dock content.
    pub fn objects_dock(&self) -> &Entity<ObjectsDock> {
        &self.objects
    }

    /// The contacts dock content.
    pub fn contacts_dock(&self) -> &Entity<ContactsDock> {
        &self.contacts
    }

    /// Opens the contacts dock and moves focus to its add-contact field.
    pub fn open_add_contact(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_tool(DockTool::Contacts, window, cx);
        self.contacts.update(cx, |dock, cx| dock.focus_add(window, cx));
    }

    /// The hold-to-confirm strip.
    pub fn confirm_strip(&self, cx: &App) -> Entity<ConfirmStrip> {
        self.root.read(cx).confirm_strip().clone()
    }

    /// The open dock tool.
    pub fn open_tool_kind(&self) -> Option<DockTool> {
        self.open_tool
    }

    /// The pinned dock tool.
    pub fn pinned_tool(&self) -> Option<DockTool> {
        self.prefs.pinned
    }

    /// The whole dock layout as the dock engine dumps it (what persists).
    pub fn dock_layout(&self, cx: &App) -> gpui_kit::component::dock::DockAreaState {
        self.dock_area.read(cx).dump(cx)
    }

    /// Shows a context, or nothing. A summoned (unpinned) tool closes.
    pub fn select_context(
        &mut self,
        id: Option<ContextId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let changed = self.current != id;
        self.current = id.clone();
        let sink =
            id.as_ref().map(|id| IntentRoot::sink(&self.root).scoped(Scope::Context(id.clone())));
        if changed {
            self.store.update(cx, |store, _| store.focus(id.as_ref()));
        }
        self.conversation.update(cx, |field, cx| field.set_context(id.clone(), cx));
        self.sync_composer(cx);
        self.composer.update(cx, |composer, cx| composer.set_sink(sink, window, cx));
        self.stack.update(cx, |stack, cx| stack.set_current(id.clone(), cx));
        self.members.update(cx, |dock, cx| dock.set_context(id.clone(), cx));
        self.evidence.update(cx, |dock, cx| dock.set_context(id, cx));
        if changed
            && let Some(open) = self.open_tool
            && self.prefs.pinned != Some(open)
        {
            self.close_dock(window, cx);
        }
        cx.notify();
    }

    /// Tells the composer what the backend cannot do in the shown context.
    fn sync_composer(&mut self, cx: &mut Context<Self>) {
        let unavailable = {
            let store = self.store.read(cx).store();
            let kind = self.current.as_ref().and_then(|id| store.context(id)).map(|c| c.kind);
            composer::Unavailable {
                message: kind.and_then(|kind| {
                    crate::i18n::unsupported(
                        store,
                        &methods::in_context(methods::SEND_MESSAGE, kind),
                    )
                }),
                task: crate::i18n::unsupported(store, methods::TASK_START),
            }
        };
        self.composer.update(cx, |composer, cx| composer.set_unavailable(unavailable, cx));
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

    /// "Take me to what needs me": moves to the next attention item after
    /// the highlighted one, across contexts and accounts, switching the
    /// rendered account when the next item lives in another one.
    pub fn go_to_needs_me(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (items, active) = {
            let handle = self.store.read(cx);
            (handle.attention_across_accounts(), handle.active_account().cloned())
        };
        let highlighted = self.conversation.read(cx).highlighted().cloned();
        let Some(next) = next_attention(&items, active.as_ref(), highlighted.as_ref()) else {
            return;
        };
        if active.as_ref() != Some(&next.account) {
            self.store.update(cx, |store, cx| store.set_active_account(&next.account, cx));
        }
        if self.current.as_ref() != Some(&next.item.context_id) {
            self.select_context(Some(next.item.context_id.clone()), window, cx);
        }
        let moment = next.item.moment_id.clone();
        self.conversation.update(cx, |field, cx| {
            field.focus_moment(&moment, cx);
            field.focus_highlighted(window, cx);
        });
    }

    /// The pin command for the current context.
    ///
    /// Takes the entity: the field emits through the root sink, and the
    /// workspace must not be borrowed while it does.
    pub fn toggle_pin(workspace: &Entity<Self>, window: &mut Window, cx: &mut App) {
        let field = workspace.read(cx).conversation.clone();
        field.update(cx, |field, cx| field.toggle_pin(window, cx));
    }

    fn panel_id(&self, tool: DockTool, cx: &App) -> Option<gpui_kit::component::dock::PanelId> {
        self.tools.iter().find(|slot| slot.tool == tool).map(|slot| slot.handle.panel_id(cx))
    }

    /// Opens a tool (summoned), selecting its tab.
    pub fn open_tool(&mut self, tool: DockTool, window: &mut Window, cx: &mut Context<Self>) {
        let panel = self.panel_id(tool, cx);
        self.dock_area.update(cx, |area, cx| {
            if !area.is_dock_open(DockPlacement::Right) {
                area.toggle_dock(DockPlacement::Right, window, cx);
            }
            if let Some(panel) = panel {
                area.select_panel(panel, window, cx);
            }
        });
        self.open_tool = Some(tool);
        cx.notify();
    }

    fn close_dock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dock_area.update(cx, |area, cx| {
            if area.is_dock_open(DockPlacement::Right) {
                area.toggle_dock(DockPlacement::Right, window, cx);
            }
        });
        self.open_tool = None;
        cx.notify();
    }

    /// Opens a tool, or closes it when it is already open.
    pub fn toggle_tool(&mut self, tool: DockTool, window: &mut Window, cx: &mut Context<Self>) {
        if self.open_tool == Some(tool) {
            self.close_dock(window, cx);
        } else {
            self.open_tool(tool, window, cx);
        }
    }

    /// The keyboard path to a tool: opens it and moves focus into its panel
    /// (Escape closes it again), or closes it and returns to the composer.
    pub fn toggle_tool_with_focus(
        &mut self,
        tool: DockTool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.open_tool == Some(tool) {
            self.close_dock(window, cx);
            self.composer.update(cx, |composer, cx| composer.focus(window, cx));
            return;
        }
        self.open_tool(tool, window, cx);
        if let Some(slot) = self.tools.iter().find(|slot| slot.tool == tool) {
            let focus = slot.panel.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
        }
    }

    /// Shows the `ix`-th context of the stack (secondary-1 to secondary-6)
    /// and moves focus to the composer.
    pub fn select_stack_entry(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let target = {
            let store = self.store.read(cx).store();
            self.stack.read(cx).entry_ids(store).into_iter().nth(ix)
        };
        let Some(id) = target else { return };
        self.select_context(Some(id), window, cx);
        self.composer.update(cx, |composer, cx| composer.focus(window, cx));
    }

    /// Pins a tool (it stays open across contexts and launches) or unpins it.
    pub fn toggle_pinned(&mut self, tool: DockTool, cx: &mut Context<Self>) {
        self.prefs.pinned = if self.prefs.pinned == Some(tool) { None } else { Some(tool) };
        self.prefs_store.save(&self.prefs);
        self.sync_pins(cx);
        cx.notify();
    }

    fn sync_pins(&mut self, cx: &mut Context<Self>) {
        let pinned = self.prefs.pinned;
        for slot in &self.tools {
            slot.panel.update(cx, |panel, cx| panel.set_pinned(pinned == Some(slot.tool), cx));
        }
    }

    /// Submits an intent proposed by an agent participant (developer panel,
    /// tests). It follows the same policy and never carries a confirmation.
    pub fn submit_agent_intent(
        &mut self,
        agent: ParticipantId,
        kind: IntentKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.root.update(cx, |root, cx| root.submit_agent(agent, kind, window, cx));
    }
}

fn dock_sink(sink: &EventSink, tool: DockTool) -> EventSink {
    sink.scoped(Scope::Panel(tool.scope_name()))
}

/// The attention item after the highlighted one (wrapping), or the first.
pub fn next_attention<'a>(
    items: &'a [AccountAttention],
    active: Option<&ramflux_desktop_core::AccountId>,
    highlighted: Option<&ramflux_desktop_core::MomentId>,
) -> Option<&'a AccountAttention> {
    let position = items.iter().position(|entry| {
        Some(&entry.account) == active && Some(&entry.item.moment_id) == highlighted
    });
    match position {
        Some(ix) => items.get((ix + 1) % items.len()),
        None => items.first(),
    }
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
