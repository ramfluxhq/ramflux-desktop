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

use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::setting::{SettingGroup, SettingItem, SettingPage};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Context, Entity, Subscription, Window, div};
use ramflux_desktop_core::{AgentAccessMode, AgentRuntimeAction, Intent, IntentKind, Store};

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

/// A runtime action the panel submitted and whose answer has not arrived.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct HostAction {
    /// The number the action was submitted with (its answer carries it).
    pub nonce: u64,
    /// Whether the action is an install (it shows the installing state).
    pub install: bool,
    /// The store's failure count when the action was submitted: a later
    /// count means the action failed, and no answer will come.
    pub failures: u64,
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
    /// The last number given to a runtime action.
    host_nonce: u64,
    /// The runtime action submitted and not answered yet.
    host_action: Option<HostAction>,
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
            host_nonce: 0,
            host_action: None,
            wizard: None,
            wizard_inputs,
            runtime_inputs,
            persona_inputs,
            form: None,
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
    /// answer, and follows the runtime action the panel submitted.
    fn follow(&mut self, cx: &mut Context<Self>) {
        let store = self.store.read(cx).store();
        if let Some(sent) = self.wizard.as_mut().and_then(|wizard| wizard.sent.as_mut()) {
            sent.observe(store);
        }
        self.follow_host(cx);
    }

    /// Gives the next runtime action its number.
    fn next_host_nonce(&mut self) -> u64 {
        self.host_nonce += 1;
        self.host_nonce
    }

    /// Follows the runtime action submitted last. Only its own answer (the
    /// status carrying its number) ends the wait, and only that answer can
    /// adopt the runtime: after an install or a restart answered running,
    /// the owned daemon restarts with the runtime's state directory so it
    /// adopts the runtime. An action that failed ends its wait too.
    fn follow_host(&mut self, cx: &mut Context<Self>) {
        let Some(action) = self.host_action else { return };
        let store = self.state(cx);
        let answer = store.agent_host().filter(|host| host.nonce == action.nonce).cloned();
        let failed = store.failures_total() > action.failures;
        if let Some(host) = answer {
            self.host_action = None;
            if let Some(state_dir) = host.adoption_for(action.nonce).map(Path::to_path_buf) {
                self.adopt_with(state_dir, cx);
            }
        } else if failed {
            self.host_action = None;
            cx.notify();
        }
    }

    /// Restarts the owned daemon with the runtime's state directory.
    fn adopt_with(&mut self, state_dir: PathBuf, cx: &mut Context<Self>) {
        self.launcher.update(cx, |launcher, cx| launcher.restart_for_runtime(Some(state_dir), cx));
    }

    /// Records a runtime action just submitted as the one being followed.
    fn arm_host_action(&mut self, nonce: u64, install: bool, cx: &mut Context<Self>) {
        let failures = self.state(cx).failures_total();
        self.host_action = Some(HostAction { nonce, install, failures });
    }

    /// Whether an install is running: submitted, not answered. While the
    /// owner still has to confirm it (the confirm strip holds it), it is not
    /// running yet.
    fn installing(&self, cx: &App) -> bool {
        let awaiting_confirmation = self.root.read(cx).pending_intent().is_some_and(|intent| {
            matches!(
                intent.kind,
                IntentKind::AgentRuntime { action: AgentRuntimeAction::Install, .. }
            )
        });
        self.host_action.is_some_and(|action| action.install) && !awaiting_confirmation
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
