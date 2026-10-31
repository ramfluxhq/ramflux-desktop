// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Settings · AI participants (DESK-AGENT-01): creating a headless AI
//! account the open account custodies, the list of those accounts, and
//! exporting an AI participant's one-shot credentials for the agent host.
//!
//! What is shown comes from the store (the accounts the daemon lists, their
//! kind and, where known, their custodian; the last export of each); this
//! panel holds only what the owner is looking at and typing (view-local):
//! whether the creation ceremony is open, which AI participant's export
//! form is open and the location typed in it.
//!
//! The creation is the account ceremony's own screens
//! ([`crate::entry::ceremony::CeremonyPanel`] given a custodian): the
//! recovery phrase is shown once, covered and erased exactly as there, and
//! nothing here ever holds it. The export asks the daemon to write the
//! bundle; this panel never sees the credentials, only whether the file was
//! written and where.
//!
//! An AI participant is never unlocked from here (or anywhere in this app):
//! the agent host starts it from the exported credentials, and the list
//! says so.
//!
//! - `rows`: the list and the export form.

mod rows;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::setting::{SettingGroup, SettingItem, SettingPage};
use gpui_kit::{App, AppContext as _, Context, Entity, Subscription, Window};
use ramflux_desktop_core::{
    Account, AccountId, AccountKind, AgentCustodyProblem, AgentExport, AgentExportPhase,
    AgentExportRefusal, Intent, IntentKind, check_agent_custodian, check_agent_export,
};

use crate::app::{StoreEvent, StoreHandle};
use crate::entry::{CeremonyEvent, CeremonyPanel};
use crate::i18n::tr;
use crate::root::IntentRoot;

/// A part of the section, rendered by its setting item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    Create,
    List,
}

/// An export sent and not answered yet.
struct Awaiting {
    agent: AccountId,
    /// The record before the request: a different one is the answer.
    before: Option<AgentExport>,
    /// The failed effects counted then: a newer failure is the answer too.
    failures: u64,
}

/// The AI participants section's view state.
pub struct AgentsPanel {
    store: Entity<StoreHandle>,
    root: Entity<IntentRoot>,
    ceremony: Entity<CeremonyPanel>,
    /// The creation ceremony is open in place of the introduction.
    creating: bool,
    /// Where the credentials are written.
    location: Entity<InputState>,
    /// The AI participant whose export form is open.
    exporting: Option<AccountId>,
    /// The export that was sent.
    awaiting: Option<Awaiting>,
    /// Why the last export was not even sent (checked here first).
    unsent: Option<(AccountId, AgentExportRefusal)>,
    _subscriptions: Vec<Subscription>,
}

impl AgentsPanel {
    /// The panel over `store`, submitting through `root`.
    pub fn new(
        store: Entity<StoreHandle>,
        root: Entity<IntentRoot>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let owner = store.read(cx).active_account().cloned();
        let ceremony =
            cx.new(|cx| CeremonyPanel::for_agent(store.clone(), root.clone(), owner, window, cx));
        let location =
            crate::i18n::localized_input("agent.export.where", window, cx, |input| input);
        let subscriptions = vec![
            cx.subscribe(&store, |this, _, event: &StoreEvent, cx| {
                if matches!(event, StoreEvent::Changed(_)) {
                    this.follow(cx);
                    cx.notify();
                }
            }),
            cx.observe(&ceremony, |_, _, cx| cx.notify()),
            cx.subscribe(&ceremony, |this, _, event: &CeremonyEvent, cx| {
                if matches!(event, CeremonyEvent::Closed) {
                    this.creating = false;
                    cx.notify();
                }
            }),
            cx.subscribe(&location, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
        ];
        Self {
            store,
            root,
            ceremony,
            creating: false,
            location,
            exporting: None,
            awaiting: None,
            unsent: None,
            _subscriptions: subscriptions,
        }
    }

    /// The creation ceremony's panel.
    pub fn ceremony(&self) -> &Entity<CeremonyPanel> {
        &self.ceremony
    }

    /// The location input of the export form.
    pub fn location_input(&self) -> &Entity<InputState> {
        &self.location
    }

    /// Whether the creation ceremony is open.
    pub fn is_creating(&self) -> bool {
        self.creating
    }

    /// The AI participant whose export form is open.
    pub fn exporting(&self) -> Option<&AccountId> {
        self.exporting.as_ref()
    }

    /// Whether an export was sent and has no answer yet.
    pub fn is_exporting(&self) -> bool {
        self.awaiting.is_some()
    }

    /// The account that would custody a new AI participant and export the
    /// credentials: the open account.
    pub(crate) fn owner(&self, cx: &App) -> Option<AccountId> {
        self.store.read(cx).active_account().cloned()
    }

    /// Why the open account cannot custody (`None`: it can).
    pub(crate) fn owner_problem(&self, cx: &App) -> Option<AgentCustodyProblem> {
        match self.owner(cx) {
            None => Some(AgentCustodyProblem::Unknown),
            Some(owner) => check_agent_custodian(self.store.read(cx).entry_store(), &owner).err(),
        }
    }

    /// The AI participants the daemon lists, by name.
    pub(crate) fn agents(&self, cx: &App) -> Vec<Account> {
        let mut agents: Vec<Account> = self
            .store
            .read(cx)
            .entry_store()
            .accounts()
            .filter(|account| account.kind == AccountKind::HeadlessAi)
            .cloned()
            .collect();
        agents.sort_by(|a, b| a.label.cmp(&b.label).then_with(|| a.id.cmp(&b.id)));
        agents
    }

    /// Ends the wait once the export has an answer.
    fn follow(&mut self, cx: &App) {
        let store = self.store.read(cx).entry_store();
        let answered = self.awaiting.as_ref().is_some_and(|awaiting| {
            store.agent_export(&awaiting.agent) != awaiting.before.as_ref()
                || store.failures_total() > awaiting.failures
        });
        if answered {
            self.awaiting = None;
        }
    }

    /// Opens the creation ceremony for the open account.
    pub fn begin_creating(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let owner = self.owner(cx);
        self.ceremony.update(cx, |ceremony, cx| {
            ceremony.set_custodian(owner, cx);
            ceremony.open(window, cx);
        });
        self.creating = true;
        cx.notify();
    }

    /// Opens the export form of `agent`, with a location to start from in
    /// the owner's home folder.
    pub fn open_export(&mut self, agent: AccountId, window: &mut Window, cx: &mut Context<Self>) {
        let name = self
            .store
            .read(cx)
            .entry_store()
            .account(&agent)
            .map_or_else(|| agent.to_string(), |account| account.label.clone());
        let suggestion = suggested_location(&name, &agent);
        self.location.update(cx, |input, cx| input.set_value(suggestion, window, cx));
        self.exporting = Some(agent);
        self.unsent = None;
        cx.notify();
    }

    /// Closes the export form.
    pub fn close_export(&mut self, cx: &mut Context<Self>) {
        self.exporting = None;
        self.unsent = None;
        cx.notify();
    }

    /// Asks the daemon to write the credentials of the open form's AI
    /// participant where the owner typed. Checked here first, with the
    /// policy's own rule, so a request the daemon would refuse is not sent.
    pub fn export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(agent), Some(owner)) = (self.exporting.clone(), self.owner(cx)) else { return };
        if self.awaiting.is_some() {
            return;
        }
        let path = self.location.read(cx).value().trim().to_owned();
        let store = self.store.read(cx).entry_store();
        if let Err(refusal) = check_agent_export(store, &owner, &agent, &path) {
            self.unsent = Some((agent, refusal));
            cx.notify();
            return;
        }
        self.unsent = None;
        let awaiting = Awaiting {
            before: store.agent_export(&agent).cloned(),
            failures: store.failures_total(),
            agent: agent.clone(),
        };
        let kind = IntentKind::ExportAgentCredentials { owner_id: owner, account_id: agent, path };
        let sent = self.root.update(cx, |root, cx| root.submit(Intent::human(kind), window, cx));
        if sent.starts_with("submitted") {
            self.awaiting = Some(awaiting);
        }
        cx.notify();
    }

    /// Chooses the location in the platform's save dialog.
    pub fn choose_location(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(agent) = self.exporting.clone() else { return };
        let suggested = file_name(&agent.to_string());
        let input = self.location.clone();
        crate::file_dialog::save_as(
            crate::file_dialog::StartDir::Documents,
            &suggested,
            window,
            cx,
            move |path, window, cx| input.update(cx, |input, cx| input.set_value(path, window, cx)),
        );
    }

    /// The last export attempt of `agent` as the owner sees it: the daemon's
    /// record, else why it was not sent.
    pub(crate) fn outcome(&self, agent: &AccountId, cx: &App) -> Option<AgentExportPhase> {
        if let Some((unsent, refusal)) = &self.unsent
            && unsent == agent
        {
            return Some(AgentExportPhase::Refused(*refusal));
        }
        self.store.read(cx).entry_store().agent_export(agent).map(|export| export.phase.clone())
    }

    /// The location of the last export of `agent`.
    pub(crate) fn exported_path(&self, agent: &AccountId, cx: &App) -> Option<String> {
        self.store.read(cx).entry_store().agent_export(agent).map(|export| export.path.clone())
    }

    fn render_part(
        &mut self,
        part: Part,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        match part {
            Part::Create => self.render_create(window, cx),
            Part::List => self.render_list(window, cx),
        }
    }
}

/// A file name for an AI participant's credentials.
fn file_name(name: &str) -> String {
    let safe: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect();
    format!("ramflux-agent-{}.bundle", safe.trim_matches('-'))
}

/// A location to start from: the owner's home folder.
fn suggested_location(name: &str, agent: &AccountId) -> String {
    let file = file_name(if name.trim().is_empty() { agent.as_str() } else { name });
    match std::env::var_os("HOME") {
        Some(home) => std::path::Path::new(&home).join(file).to_string_lossy().into_owned(),
        None => file,
    }
}

fn item(panel: &Entity<AgentsPanel>, part: Part) -> SettingItem {
    let panel = panel.clone();
    SettingItem::render(move |_, window, cx: &mut App| {
        panel.update(cx, |panel, cx| panel.render_part(part, window, cx))
    })
}

/// The AI participants section.
pub fn page(panel: &Entity<AgentsPanel>) -> SettingPage {
    SettingPage::new(tr("settings.section.agents"))
        .group(
            SettingGroup::new().title(tr("agent.settings.title")).item(item(panel, Part::Create)),
        )
        .group(
            SettingGroup::new()
                .title(tr("agent.settings.list_title"))
                .item(item(panel, Part::List)),
        )
}
