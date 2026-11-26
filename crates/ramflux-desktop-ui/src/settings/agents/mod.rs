// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Settings · Agents (AGENT-IDENTITY-01): the PRX runtime on this computer,
//! the new-agent wizard (D33–D38), the agents this account owns (persona,
//! access, PRX jobs, removal) and the features the build shows as
//! unavailable, each with its typed reason.
//!
//! What is shown comes from the store (the agent records, the creation
//! requests, the runtime status, the jobs). This panel holds only what the
//! owner is typing and which form is open. The provider key is typed into a
//! masked field, handed once to the runtime install and never kept here.
//!
//! - `wizard`: the new-agent wizard and the creation states (D38).
//! - `runtime`: the PRX runtime panel (install, status, restart, stop,
//!   uninstall).
//! - `rows`: the list of agents and their forms.

mod rows;
mod runtime;
pub mod wizard;

use std::time::Duration;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::setting::{SettingGroup, SettingItem, SettingPage};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Context, Entity, Subscription, Task, Window, div};
use ramflux_desktop_core::{AgentAccessMode, AgentHostState, Intent, IntentKind, Store};
use ramflux_desktop_sdk::plan::agents::{CREATION_POLL, pending_creation_reads};
use ramflux_desktop_sdk::plan::fetch::Fetch;

use crate::app::{StoreEvent, StoreHandle};
use crate::i18n::tr;
use crate::launch::DaemonLauncher;
use crate::root::IntentRoot;

/// A part of the section, rendered by its setting item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    Runtime,
    Create,
    List,
    Unavailable,
}

/// The persona inputs of the edit form (name and responsibility).
pub(super) struct PersonaInputs {
    pub name: Entity<InputState>,
    pub responsibility: Entity<InputState>,
}

/// The form open under one agent's row.
pub(super) enum Form {
    /// Edit the persona (D08): name, avatar preset, responsibility.
    Edit { agent: String, glyph: Option<&'static str>, tone: Option<&'static str> },
    /// Edit who may reach the agent (D08 access).
    Access { agent: String, dm: AgentAccessMode, group_inviters: AgentAccessMode },
    /// The agent's PRX jobs (D10): pause, resume, remove.
    Jobs { agent: String },
}

/// The agents section's view state.
pub struct AgentsPanel {
    store: Entity<StoreHandle>,
    root: Entity<IntentRoot>,
    /// The local daemon launcher: restarted to adopt the runtime after an
    /// install or a restart of it.
    launcher: Entity<DaemonLauncher>,
    /// An install or a restart the panel asked for and whose applied status
    /// has not arrived yet.
    adopt_pending: bool,
    /// The new-agent wizard.
    wizard: Option<wizard::Wizard>,
    /// The wizard's typed fields.
    wizard_inputs: wizard::Inputs,
    /// The runtime install form's fields (the key is masked).
    runtime_inputs: runtime::Inputs,
    /// The persona edit form's fields.
    persona_inputs: PersonaInputs,
    /// The form open under one agent's row.
    form: Option<Form>,
    /// Polls the creations still waiting (every [`CREATION_POLL`]).
    poll: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl AgentsPanel {
    /// The panel over `store`, submitting through `root`, restarting the
    /// owned daemon through `launcher`.
    pub fn new(
        store: Entity<StoreHandle>,
        root: Entity<IntentRoot>,
        launcher: Entity<DaemonLauncher>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let wizard_inputs = wizard::Inputs::new(window, cx);
        let runtime_inputs = runtime::Inputs::new(window, cx);
        let persona_inputs = PersonaInputs {
            name: crate::i18n::localized_input("agent_wizard.name_placeholder", window, cx, |i| i),
            responsibility: crate::i18n::localized_input(
                "agent_wizard.responsibility_placeholder",
                window,
                cx,
                |i| i,
            ),
        };
        let subscriptions = vec![
            cx.subscribe(&store, |this, _, event: &StoreEvent, cx| {
                if matches!(event, StoreEvent::Changed(_)) {
                    this.follow(cx);
                    cx.notify();
                }
            }),
            cx.subscribe(&wizard_inputs.name, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
        ];
        Self {
            store,
            root,
            launcher,
            adopt_pending: false,
            wizard: None,
            wizard_inputs,
            runtime_inputs,
            persona_inputs,
            form: None,
            poll: None,
            _subscriptions: subscriptions,
        }
    }

    /// The agent state of the open account's session.
    fn state<'a>(&self, cx: &'a App) -> &'a Store {
        self.store.read(cx).store()
    }

    /// Submits an intent through the root (its confirmation strip answers
    /// the owner's yes or no, and resubmits when it is confirmed).
    fn submit(&self, kind: IntentKind, window: &mut Window, cx: &mut Context<Self>) {
        let intent = Intent::human(kind);
        self.root.update(cx, |root, cx| root.submit(intent, window, cx));
    }

    /// Ends the wait of the wizard's creation once the store holds its
    /// answer, and keeps the poll running while a creation is pending.
    fn follow(&mut self, cx: &mut Context<Self>) {
        let store = self.store.read(cx).store();
        if let Some(sent) = self.wizard.as_mut().and_then(|wizard| wizard.sent.as_mut()) {
            sent.observe(store);
        }
        self.adopt_runtime(cx);
        self.ensure_poll(cx);
    }

    /// After an install or a restart the panel asked for answered running,
    /// the owned daemon restarts with the runtime's state directory, so it
    /// adopts the runtime. A refused or stopped runtime restarts nothing.
    fn adopt_runtime(&mut self, cx: &mut Context<Self>) {
        if !self.adopt_pending {
            return;
        }
        let Some(host) = self.state(cx).agent_host().cloned() else { return };
        if !host.applied {
            return;
        }
        self.adopt_pending = false;
        let Some(state_dir) = host.state_dir else { return };
        if host.refusal.is_some() || host.state != AgentHostState::Running {
            return;
        }
        self.launcher.update(cx, |launcher, cx| launcher.restart_for_runtime(Some(state_dir), cx));
    }

    /// Starts the poll when a creation is pending and none is running.
    fn ensure_poll(&mut self, cx: &mut Context<Self>) {
        if self.poll.is_some() || pending_creation_reads(self.state(cx)).is_empty() {
            return;
        }
        self.poll = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(CREATION_POLL).await;
                let keep = this.update(cx, |panel, cx| panel.poll_creations(cx)).unwrap_or(false);
                if !keep {
                    break;
                }
            }
        }));
    }

    /// One poll tick: reads every creation still waiting (`agent.operation.get`).
    /// Returns whether the poll goes on.
    fn poll_creations(&mut self, cx: &mut Context<Self>) -> bool {
        let reads = pending_creation_reads(self.state(cx));
        if reads.is_empty() {
            self.poll = None;
            cx.notify();
            return false;
        }
        for read in reads {
            if let Fetch::AgentOperation(request_id) = read {
                let kind = IntentKind::AgentOperationRefresh { request_id };
                self.store.update(cx, |store, cx| store.dispatch(Intent::human(kind), cx));
            }
        }
        cx.notify();
        true
    }

    /// The last failed agent request, as its translated message.
    fn last_failure(&self, cx: &App) -> Option<String> {
        self.state(cx)
            .failures()
            .iter()
            .rev()
            .find(|failure| {
                ramflux_desktop_sdk::plan::agents::is_agent_method(&failure.method)
                    || failure.method == ramflux_desktop_sdk::plan::agents::RUNTIME_METHOD
            })
            .map(|failure| failure.message.clone())
    }

    /// The form open under an agent's row. The jobs form reads the agent's
    /// PRX jobs as it opens.
    fn open_form(&mut self, form: Option<Form>, cx: &mut Context<Self>) {
        let jobs_of = match &form {
            Some(Form::Jobs { agent }) => Some(agent.clone()),
            _ => None,
        };
        self.form = form;
        if let Some(agent) = jobs_of {
            let kind = IntentKind::AgentJobsRefresh { agent };
            self.store.update(cx, |store, cx| store.dispatch(Intent::human(kind), cx));
        }
        cx.notify();
    }

    /// One part of the page.
    fn render_part(&mut self, part: Part, cx: &mut Context<Self>) -> AnyElement {
        match part {
            Part::Runtime => self.render_runtime(cx),
            Part::Create => self.render_create(cx),
            Part::List => self.render_list(cx),
            Part::Unavailable => self.render_unavailable(cx),
        }
    }
}

fn item(panel: &Entity<AgentsPanel>, part: Part) -> SettingItem {
    let panel = panel.clone();
    SettingItem::render(move |_, _, cx: &mut App| {
        panel.update(cx, |panel, cx| panel.render_part(part, cx))
    })
}

/// The agents section.
pub fn page(panel: &Entity<AgentsPanel>) -> SettingPage {
    SettingPage::new(tr("settings.section.agents"))
        .group(
            SettingGroup::new().title(tr("agents.runtime.title")).item(item(panel, Part::Runtime)),
        )
        .group(SettingGroup::new().item(item(panel, Part::Create)))
        .group(
            SettingGroup::new()
                .title(tr("agent.settings.list_title"))
                .item(item(panel, Part::List)),
        )
        .group(
            SettingGroup::new()
                .title(tr("agents.unavailable.title"))
                .item(item(panel, Part::Unavailable)),
        )
}

/// A muted line of text.
pub(super) fn muted(text: gpui_kit::SharedString, cx: &App) -> AnyElement {
    use gpui_kit::component::ActiveTheme as _;
    use ramflux_desktop_navigation::scale::text_secondary;
    div()
        .text_size(text_secondary())
        .text_color(cx.theme().muted_foreground)
        .whitespace_normal()
        .child(text)
        .into_any_element()
}

/// Whether the grace period of a request has passed (unix seconds).
pub(super) fn seconds_left(expires_at: Option<u64>) -> Option<Duration> {
    let expires = expires_at?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    Some(Duration::from_secs(expires.saturating_sub(now)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_without_an_expiry_has_no_countdown() {
        assert_eq!(seconds_left(None), None);
    }

    #[test]
    fn a_passed_expiry_leaves_no_time() {
        assert_eq!(seconds_left(Some(1)), Some(Duration::ZERO));
    }
}
