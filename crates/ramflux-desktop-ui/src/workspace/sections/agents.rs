// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The Agents section (design 04 / D01): the roster of the agents this
//! account talks to in the list pane, the conversation with the chosen one
//! in the detail pane, and "Start a multi-agent conversation" at the foot
//! (a new group, the existing feature). The roster is the account's agent
//! contacts; nothing here creates, configures or authorizes an agent.

use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::InputEvent;
use gpui_kit::component::{Selectable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, Entity, EventEmitter, IntoElement, Render, SharedString, Subscription, Window,
    div,
};
use ramflux_desktop_core::{
    ContextKind, MomentKind, ParticipantId, ParticipantKind, Store, TaskState, selectors,
};

use super::directory::{self, Search};
use crate::actions::{NextConversation, PreviousConversation};
use crate::app::{StoreEvent, StoreHandle};
use crate::i18n::tr;

/// What the roster reports to the workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentRosterEvent {
    /// Open the conversation with this agent.
    Selected(ParticipantId),
    /// Open the agents' management page (create an agent).
    Manage,
    /// Start a conversation with several agents (a new group).
    NewGroup,
}

/// The roster's view state.
pub struct AgentRoster {
    store: Entity<StoreHandle>,
    search: Search,
    selected: Option<ParticipantId>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<AgentRosterEvent> for AgentRoster {}

/// One listed agent.
pub struct AgentEntry {
    /// The agent.
    pub id: ParticipantId,
    /// Its name, cleaned for one line.
    pub name: SharedString,
    /// What it is ("Agent").
    pub role: SharedString,
    /// Where things stand with it: a task waits for the user, the private
    /// conversation, or none yet.
    pub status: SharedString,
}

impl AgentRoster {
    /// A roster over the rendered account's store.
    pub fn new(store: Entity<StoreHandle>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = Search::new("sections.agents.search", window, cx);
        let subscriptions = vec![
            cx.subscribe(&store, |_, _, event: &StoreEvent, cx| {
                if let StoreEvent::Changed(changes) = event
                    && (changes.touches_docks() || changes.touches_status())
                {
                    cx.notify();
                }
            }),
            cx.subscribe(search.input(), |this, _, event: &InputEvent, cx| match event {
                InputEvent::Change => {
                    this.search.active = 0;
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => this.pick_active(cx),
                _ => {}
            }),
            crate::i18n::on_locale_switch(cx, |_, cx| cx.notify()),
        ];
        Self { store, search, selected: None, _subscriptions: subscriptions }
    }

    /// Marks the agent whose conversation is shown.
    pub fn set_selected(&mut self, id: Option<ParticipantId>, cx: &mut Context<Self>) {
        if self.selected != id {
            self.selected = id;
            cx.notify();
        }
    }

    /// The agent shown, if any.
    pub fn selected(&self) -> Option<&ParticipantId> {
        self.selected.as_ref()
    }

    /// Focuses the search field.
    pub fn focus_search(&self, window: &mut Window, cx: &mut App) {
        self.search.focus(window, cx);
    }

    /// The listed agents, in display order.
    pub fn entries(&self, cx: &App) -> Vec<AgentEntry> {
        let store = self.store.read(cx).store();
        let query = self.search.query(cx);
        let mut agents: Vec<AgentEntry> = selectors::contacts(store)
            .into_iter()
            .filter(|p| p.kind == ParticipantKind::Agent)
            .filter(|p| directory::matches(&query, &[&p.display, p.id.as_str()]))
            .map(|p| AgentEntry {
                id: p.id.clone(),
                name: ramflux_desktop_navigation::row::list_text(&p.display),
                role: tr("sections.agents.role"),
                status: status(store, &p.id),
            })
            .collect();
        agents.sort_by_key(|a| a.name.to_lowercase());
        agents
    }

    fn pick_active(&mut self, cx: &mut Context<Self>) {
        let entries = self.entries(cx);
        if let Some(entry) = entries.get(self.search.active.min(entries.len().saturating_sub(1))) {
            cx.emit(AgentRosterEvent::Selected(entry.id.clone()));
        }
    }

    fn step(&mut self, step: isize, cx: &mut Context<Self>) {
        let len = self.entries(cx).len();
        self.search.step(step, len);
        cx.notify();
    }
}

/// Where things stand with `agent`: a task of theirs waits for the user,
/// else the private conversation exists, else none yet.
pub(crate) fn status(store: &Store, agent: &ParticipantId) -> SharedString {
    let waiting = store.tasks().any(|t| {
        &t.assignee == agent && matches!(t.state, TaskState::NeedsMe | TaskState::Proposed)
    });
    if waiting {
        return tr("sections.agents.task_waiting");
    }
    let direct =
        selectors::contexts_with(store, agent).into_iter().find(|c| c.kind == ContextKind::Direct);
    match direct {
        Some(context)
            if store
                .context_moments(&context.id)
                .any(|m| matches!(m.kind, MomentKind::Message(_))) =>
        {
            tr("sections.agents.private")
        }
        Some(_) => tr("sections.agents.private_empty"),
        None => tr("sections.agents.no_conversation"),
    }
}

impl Render for AgentRoster {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = ramflux_desktop_navigation::palette(cx);
        let entries = self.entries(cx);
        let listed = {
            let store = self.store.read(cx).store();
            store.is_listed(ramflux_desktop_core::Listing::Contacts)
        };
        let empty = (listed && entries.is_empty()).then(|| {
            directory::placeholder(
                "agents-empty",
                tr(if self.search.query(cx).is_empty() {
                    "sections.agents.empty"
                } else {
                    "sections.no_match"
                }),
                tr("sections.agents.empty_help"),
                cx,
            )
        });
        let active = self.search.cursor(window, cx);
        let rem = |px: f32| gpui_kit::rems(px / 15.0);
        v_flex()
            .id("agent-roster")
            .test_support()
            .key_context("ConversationList")
            .on_action(cx.listener(|this, _: &NextConversation, _, cx| this.step(1, cx)))
            .on_action(cx.listener(|this, _: &PreviousConversation, _, cx| this.step(-1, cx)))
            .size_full()
            .min_w_0()
            .bg(palette.nav)
            .border_r_1()
            .border_color(palette.line)
            .px_3()
            .pt(rem(24.0))
            .pb(rem(20.0))
            .gap_4()
            .child(directory::header(
                tr("navigation.agents"),
                Button::new("agents-add")
                    .small()
                    .ghost()
                    .icon(IconName::Plus)
                    .accessibility_label(tr("sections.agents.add"))
                    .tooltip(tr("sections.agents.add"))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(AgentRosterEvent::Manage))),
                cx,
            ))
            .child(self.search.field("agents-search", tr("sections.agents.search"), cx))
            .children(empty)
            .child(
                v_flex()
                    .id("agents-list")
                    .role(gpui_kit::Role::List)
                    .aria_label(tr("navigation.agents"))
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .gap_4()
                    .children(entries.into_iter().enumerate().map(|(ix, entry)| {
                        let selected = self.selected.as_ref() == Some(&entry.id);
                        let pick = entry.id.clone();
                        Button::new(SharedString::from(format!("agent-{}", entry.id)))
                            .ghost()
                            .w_full()
                            .h_auto()
                            .px_3()
                            .py_3()
                            .rounded(gpui_kit::px(palette.radius_sm))
                            .selected(selected)
                            .toggled(selected)
                            .when(active == Some(ix), |row| {
                                row.border_1().border_color(palette.focus)
                            })
                            .accessibility_label(entry.name.clone())
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.emit(AgentRosterEvent::Selected(pick.clone()))
                            }))
                            .child(
                                h_flex()
                                    .w_full()
                                    .gap_3()
                                    .child(
                                        div()
                                            .flex_none()
                                            .size(rem(32.0))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .rounded(rem(palette.radius_md))
                                            .bg(palette.agent_av)
                                            .text_color(palette.on_agent_av)
                                            .child(ramflux_desktop_navigation::row::initial(
                                                &entry.name,
                                            )),
                                    )
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .min_w_0()
                                            .child(
                                                div()
                                                    .truncate()
                                                    .text_size(rem(palette.size_body))
                                                    .font_weight(gpui_kit::FontWeight::MEDIUM)
                                                    .child(entry.name.clone()),
                                            )
                                            .child(
                                                div()
                                                    .text_size(rem(palette.size_meta))
                                                    .text_color(palette.secondary)
                                                    .child(entry.role.clone()),
                                            )
                                            .child(
                                                div()
                                                    .truncate()
                                                    .text_size(rem(palette.size_caption))
                                                    .text_color(palette.secondary)
                                                    .child(entry.status.clone()),
                                            ),
                                    ),
                            )
                    })),
            )
            .child(
                h_flex().child(
                    crate::style::secondary_button("agents-new-group", cx)
                        .label(tr("sections.agents.new_group"))
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(AgentRosterEvent::NewGroup))),
                ),
            )
    }
}
