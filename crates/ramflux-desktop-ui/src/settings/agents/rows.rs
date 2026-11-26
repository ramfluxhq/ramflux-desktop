// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The agents section's parts below the runtime panel: the introduction
//! with its create button (or the wizard in its place), the agents this
//! account owns with their forms (persona, access, PRX jobs, removal), and
//! the features shown as unavailable with their typed reasons.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::Selectable as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::Input;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::v_flex;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Context, ElementId, Role, SharedString, Window, div};
use ramflux_desktop_core::projection::agents::UNAVAILABLE;
use ramflux_desktop_core::{
    AgentAccess, AgentAccessMode, AgentJob, AgentJobAction, AgentJobKind, AgentLifecycle,
    AgentPersona, AgentRecord, AgentRosterState, AgentUnavailable, IntentKind,
};
use ramflux_desktop_navigation::direction::h_flex;
use ramflux_desktop_navigation::scale::{space, text_caption, text_secondary};
use ramflux_desktop_sdk::translate::agents::refusal_key;

use super::wizard::{self, GLYPHS, TONES};
use super::{AgentsPanel, Form, muted};
use crate::i18n::{tr, tr_args};
use crate::style::{DesignButton as _, RETIRED_OPACITY};

/// A muted note a reader hears and the tests read.
fn note(id: &'static str, key: &str, cx: &App) -> AnyElement {
    div()
        .id(id)
        .role(Role::Status)
        .aria_label(tr(key))
        .test_support()
        .child(muted(tr(key), cx))
        .into_any_element()
}

/// A feature shown as unavailable: a line with its typed reason. Nothing is
/// sent for it.
pub(super) fn unavailable_line(
    id: impl Into<ElementId>,
    reason: AgentUnavailable,
    cx: &App,
) -> AnyElement {
    let text = tr(reason.key());
    div()
        .id(id)
        .role(Role::Status)
        .aria_label(text.clone())
        .test_support()
        .text_size(text_caption())
        .text_color(cx.theme().muted_foreground)
        .whitespace_normal()
        .child(text)
        .into_any_element()
}

/// The agents of the open account, by name.
fn records(store: &ramflux_desktop_core::Store) -> Vec<AgentRecord> {
    let mut agents: Vec<AgentRecord> = store.agents().cloned().collect();
    agents.sort_by(|a, b| {
        a.persona.name.cmp(&b.persona.name).then_with(|| a.agent_anchor.cmp(&b.agent_anchor))
    });
    agents
}

fn access_text(access: &AgentAccess) -> SharedString {
    tr_args(
        "agents.row.access",
        &[
            ("dm", &tr(wizard::access_key(access.dm))),
            ("group", &tr(wizard::access_key(access.group_inviters))),
        ],
    )
}

fn job_kind_key(kind: AgentJobKind) -> &'static str {
    match kind {
        AgentJobKind::Cron => "agents.job.kind.cron",
        AgentJobKind::XinTask => "agents.job.kind.xin_task",
        AgentJobKind::XinGoal => "agents.job.kind.xin_goal",
    }
}

/// The glyph and tone of a record's avatar, as the picker's static values.
fn picked(avatar: &str) -> (Option<&'static str>, Option<&'static str>) {
    let Some((glyph, tone)) = wizard::preset_parts(avatar) else { return (None, None) };
    (
        GLYPHS.iter().copied().find(|value| *value == glyph),
        TONES.iter().copied().find(|value| *value == tone),
    )
}

impl AgentsPanel {
    /// Why the owner cannot create an agent now (translated), if so.
    fn create_reason(&self, cx: &App) -> Option<SharedString> {
        let owner = self.owner(cx)?;
        let problem =
            ramflux_desktop_core::check_agent_custodian(self.store.read(cx).entry_store(), &owner)
                .err();
        problem.map(|problem| crate::i18n::agent_custody_problem(Some(problem)))
    }

    /// The introduction and the create button; the wizard in their place
    /// while it is open.
    pub(super) fn render_create(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if self.wizard.is_some() {
            return self.render_wizard(cx);
        }
        let reason = self.create_reason(cx);
        v_flex()
            .gap(space(2.))
            .child(note("agent-intro", "agent.settings.intro", cx))
            .children(reason.clone().map(|reason| {
                div()
                    .id("agent-create-reason")
                    .role(Role::Status)
                    .aria_label(reason.clone())
                    .test_support()
                    .child(muted(reason, cx))
            }))
            .child(
                h_flex().child(crate::gate::gated(
                    Button::new("agent-create")
                        .small()
                        .design_primary(cx)
                        .design_label(tr("agent.settings.create"))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.begin_wizard(window, cx);
                        })),
                    reason,
                    cx,
                )),
            )
            .into_any_element()
    }

    /// The agents the open account owns, with their state and actions.
    pub(super) fn render_list(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let state = self.state(cx);
        let roster = match state.agent_roster() {
            AgentRosterState::Loading => Some(tr("agents.roster.loading")),
            AgentRosterState::Offline => Some(tr("agents.roster.offline")),
            AgentRosterState::Failed(refusal) => Some(tr(refusal_key(&refusal))),
            AgentRosterState::Ready => None,
        };
        let failure = self
            .last_failure(cx)
            .filter(|message| message.starts_with("backend.agent."))
            .map(|message| tr(&message));
        let agents = records(self.state(cx));
        v_flex()
            .id("agent-list")
            .role(Role::List)
            .aria_label(tr("agent.settings.list_title"))
            .test_support()
            .gap(space(3.))
            .children(roster.map(|text| muted(text, cx)))
            .children(failure.map(|text| {
                div()
                    .id("agent-last-failure")
                    .role(Role::Alert)
                    .aria_label(text.clone())
                    .test_support()
                    .child(Alert::error("agent-last-failure-alert", text).small())
            }))
            .when(agents.is_empty(), |col| {
                col.child(note("agent-empty", "agent.settings.empty", cx))
            })
            .children(agents.iter().map(|agent| self.agent_row(agent, cx)))
            .into_any_element()
    }

    fn agent_row(&mut self, agent: &AgentRecord, cx: &mut Context<Self>) -> AnyElement {
        let anchor = agent.agent_anchor.clone();
        let active = agent.lifecycle == AgentLifecycle::Active;
        let state = if active {
            Tag::success().small().child(tr("agents.row.active")).into_any_element()
        } else {
            div()
                .text_size(text_caption())
                .text_color(cx.theme().muted_foreground)
                .child(tr("agents.row.retired"))
                .into_any_element()
        };
        let label = tr_args("agents.row.label", &[("name", &agent.persona.name)]);
        let row = h_flex()
            .id(ElementId::from(format!("agent-row-{anchor}")))
            .opacity(if active { 1.0 } else { RETIRED_OPACITY })
            .role(Role::ListItem)
            .aria_label(label)
            .test_support()
            .gap(space(3.))
            .px(space(3.))
            .py(space(2.))
            .border_1()
            .border_color(cx.theme().border)
            .rounded(cx.theme().radius)
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(text_secondary())
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .child(agent.persona.name.clone()),
                    )
                    .child(
                        div()
                            .text_size(text_caption())
                            .text_color(cx.theme().muted_foreground)
                            .whitespace_normal()
                            .child(agent.persona.responsibility.clone()),
                    )
                    .child(
                        div()
                            .text_size(text_caption())
                            .text_color(cx.theme().muted_foreground)
                            .child(access_text(&agent.access)),
                    ),
            )
            .child(state)
            .children(active.then(|| self.row_actions(agent, cx)));
        let open = self.form.as_ref().is_some_and(|form| form.agent() == anchor);
        v_flex()
            .gap(space(2.))
            .child(row)
            .children(active.then(|| {
                unavailable_line("agent-pause-unavailable", AgentUnavailable::AgentPause, cx)
            }))
            .children(open.then(|| self.form_view(agent, cx)))
            .into_any_element()
    }

    /// The actions of an active agent's row.
    fn row_actions(&mut self, agent: &AgentRecord, cx: &mut Context<Self>) -> AnyElement {
        let anchor = agent.agent_anchor.clone();
        let button = |part: &'static str, key: &'static str| {
            let anchor = anchor.clone();
            (part, key, anchor)
        };
        let [edit, access, jobs, remove] = [
            button("edit", "agents.row.edit"),
            button("access", "agents.row.access_button"),
            button("jobs", "agents.row.jobs"),
            button("remove", "agents.row.remove"),
        ];
        h_flex()
            .gap(space(1.))
            .child(
                Button::new(ElementId::from(format!("agent-{}-{}", edit.0, edit.2)))
                    .small()
                    .design_secondary(cx)
                    .design_label(tr(edit.1))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(agent) = this.state(cx).agent(&edit.2).cloned() {
                            this.open_edit(&agent, window, cx);
                        }
                    })),
            )
            .child(
                Button::new(ElementId::from(format!("agent-{}-{}", access.0, access.2)))
                    .small()
                    .design_secondary(cx)
                    .design_label(tr(access.1))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(agent) = this.state(cx).agent(&access.2).cloned() {
                            this.open_access(&agent, window, cx);
                        }
                    })),
            )
            .child(
                Button::new(ElementId::from(format!("agent-{}-{}", jobs.0, jobs.2)))
                    .small()
                    .design_secondary(cx)
                    .design_label(tr(jobs.1))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(agent) = this.state(cx).agent(&jobs.2).cloned() {
                            this.open_jobs(&agent, cx);
                        }
                    })),
            )
            .child(
                Button::new(ElementId::from(format!("agent-{}-{}", remove.0, remove.2)))
                    .small()
                    .design_secondary(cx)
                    .design_label(tr(remove.1))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(agent) = this.state(cx).agent(&remove.2).cloned() {
                            this.remove(&agent, window, cx);
                        }
                    })),
            )
            .into_any_element()
    }

    /// Opens the persona form of `agent` (D08).
    fn open_edit(&mut self, agent: &AgentRecord, window: &mut Window, cx: &mut Context<Self>) {
        let (glyph, tone) = picked(&agent.persona.avatar);
        let name = agent.persona.name.clone();
        let responsibility = agent.persona.responsibility.clone();
        self.persona_inputs.name.update(cx, |input, cx| input.set_value(name, window, cx));
        self.persona_inputs
            .responsibility
            .update(cx, |input, cx| input.set_value(responsibility, window, cx));
        self.open_form(Some(Form::Edit { agent: agent.agent_anchor.clone(), glyph, tone }), cx);
    }

    /// Opens the access form of `agent` (D08 access).
    fn open_access(&mut self, agent: &AgentRecord, _: &mut Window, cx: &mut Context<Self>) {
        let form = Form::Access {
            agent: agent.agent_anchor.clone(),
            dm: agent.access.dm,
            group_inviters: agent.access.group_inviters,
        };
        self.open_form(Some(form), cx);
    }

    /// Opens the PRX jobs of `agent` (D10).
    fn open_jobs(&mut self, agent: &AgentRecord, cx: &mut Context<Self>) {
        self.open_form(Some(Form::Jobs { agent: agent.agent_anchor.clone() }), cx);
    }

    /// Removes `agent` (D11): the root asks for the owner's confirmation.
    fn remove(&mut self, agent: &AgentRecord, window: &mut Window, cx: &mut Context<Self>) {
        let kind = IntentKind::AgentDelete { agent: agent.agent_anchor.clone() };
        self.submit(kind, window, cx);
        self.open_form(None, cx);
    }

    /// Chooses an avatar glyph or tone of the open persona form.
    pub(super) fn choose_edit_avatar(
        &mut self,
        glyph: bool,
        value: &'static str,
        cx: &mut Context<Self>,
    ) {
        if let Some(Form::Edit { glyph: g, tone: t, .. }) = self.form.as_mut() {
            if glyph {
                *g = Some(value);
            } else {
                *t = Some(value);
            }
        }
        cx.notify();
    }

    /// Chooses one access mode of the open access form.
    pub(super) fn choose_form_access(
        &mut self,
        dm: bool,
        mode: AgentAccessMode,
        cx: &mut Context<Self>,
    ) {
        if let Some(Form::Access { dm: d, group_inviters: g, .. }) = self.form.as_mut() {
            if dm {
                *d = mode;
            } else {
                *g = mode;
            }
        }
        cx.notify();
    }

    /// Saves the persona form (D08): the name, the avatar and the responsibility.
    pub(super) fn save_persona(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Form::Edit { agent, glyph, tone }) = self.form.take() else { return };
        let persona = AgentPersona {
            name: self.persona_inputs.name.read(cx).value().to_string(),
            avatar: wizard::preset(glyph, tone).unwrap_or_default(),
            responsibility: self.persona_inputs.responsibility.read(cx).value().to_string(),
        };
        self.submit(IntentKind::AgentPersonaEdit { agent, persona }, window, cx);
        cx.notify();
    }

    /// Saves the access form: its modes, with the record's allowed humans.
    pub(super) fn save_access(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Form::Access { agent, dm, group_inviters }) = self.form.take() else { return };
        let allowed_humans = self
            .state(cx)
            .agent(&agent)
            .map(|record| record.access.allowed_humans.clone())
            .unwrap_or_default();
        let access = AgentAccess { dm, group_inviters, allowed_humans };
        self.submit(IntentKind::AgentAccessSet { agent, access }, window, cx);
        cx.notify();
    }

    /// One PRX job's action (pause, resume, remove).
    fn job_action(
        &mut self,
        agent: &str,
        job: &AgentJob,
        action: AgentJobAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let kind = IntentKind::AgentJob {
            agent: agent.to_owned(),
            kind: job.kind,
            id: job.id.clone(),
            action,
        };
        self.submit(kind, window, cx);
    }

    /// The form open under an agent's row.
    fn form_view(&mut self, agent: &AgentRecord, cx: &mut Context<Self>) -> AnyElement {
        let anchor = agent.agent_anchor.clone();
        let body = match self.form.as_ref() {
            Some(Form::Edit { glyph, tone, .. }) => {
                let (glyph, tone) = (*glyph, *tone);
                let ready = !self.persona_inputs.name.read(cx).value().trim().is_empty()
                    && !self.persona_inputs.responsibility.read(cx).value().trim().is_empty()
                    && wizard::preset(glyph, tone).is_some();
                v_flex()
                    .gap(space(2.))
                    .child(
                        Input::new(&self.persona_inputs.name)
                            .id("agent-edit-name")
                            .aria_label(tr("agent_wizard.name")),
                    )
                    .child(
                        Input::new(&self.persona_inputs.responsibility)
                            .id("agent-edit-responsibility")
                            .aria_label(tr("agent_wizard.responsibility")),
                    )
                    .child(self.edit_choices(true, glyph, cx))
                    .child(self.edit_choices(false, tone, cx))
                    .child(
                        h_flex()
                            .gap(space(2.))
                            .child(
                                Button::new("agent-edit-save")
                                    .small()
                                    .design_primary(cx)
                                    .design_label(tr("agents.form.save"))
                                    .design_disabled(!ready, cx)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.save_persona(window, cx)
                                    })),
                            )
                            .child(self.close_button(cx)),
                    )
                    .into_any_element()
            }
            Some(Form::Access { dm, group_inviters, .. }) => {
                let (dm, group) = (*dm, *group_inviters);
                v_flex()
                    .gap(space(2.))
                    .child(self.form_access_row(true, dm, cx))
                    .child(self.form_access_row(false, group, cx))
                    .child(
                        h_flex()
                            .gap(space(2.))
                            .child(
                                Button::new("agent-access-save")
                                    .small()
                                    .design_primary(cx)
                                    .design_label(tr("agents.form.save"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.save_access(window, cx)
                                    })),
                            )
                            .child(self.close_button(cx)),
                    )
                    .into_any_element()
            }
            Some(Form::Jobs { .. }) => {
                let jobs = self.state(cx).agent_jobs(&anchor).to_vec();
                v_flex()
                    .gap(space(2.))
                    .child(
                        div()
                            .text_size(text_secondary())
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .child(tr("agents.jobs.title")),
                    )
                    .when(jobs.is_empty(), |col| col.child(muted(tr("agents.jobs.none"), cx)))
                    .children(jobs.iter().map(|job| self.job_row(&anchor, job, cx)))
                    .child(self.close_button(cx))
                    .into_any_element()
            }
            None => div().into_any_element(),
        };
        v_flex()
            .id(ElementId::from(format!("agent-form-{anchor}")))
            .test_support()
            .gap(space(2.))
            .p(space(3.))
            .border_1()
            .border_color(cx.theme().border)
            .rounded(cx.theme().radius)
            .child(body)
            .into_any_element()
    }

    fn close_button(&mut self, cx: &mut Context<Self>) -> AnyElement {
        Button::new("agent-form-close")
            .small()
            .design_secondary(cx)
            .design_label(tr("agents.form.close"))
            .on_click(cx.listener(|this, _, _, cx| this.open_form(None, cx)))
            .into_any_element()
    }

    /// The glyph or tone picker of the persona form.
    fn edit_choices(
        &mut self,
        glyph: bool,
        chosen: Option<&'static str>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let values: &'static [&'static str] = if glyph { &GLYPHS } else { &TONES };
        h_flex()
            .gap(space(2.))
            .flex_wrap()
            .children(values.iter().map(|value| {
                let value: &'static str = value;
                let selected = chosen == Some(value);
                let prefix = if glyph { "glyph" } else { "tone" };
                Button::new(ElementId::from(format!("agent-edit-{prefix}-{value}")))
                    .small()
                    .design_secondary(cx)
                    .selected(selected)
                    .toggled(selected)
                    .design_label(tr(wizard::choice_key(prefix, value)))
                    .on_click(
                        cx.listener(move |this, _, _, cx| {
                            this.choose_edit_avatar(glyph, value, cx)
                        }),
                    )
            }))
            .into_any_element()
    }

    fn form_access_row(
        &mut self,
        dm: bool,
        chosen: AgentAccessMode,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let label = if dm { tr("agent_wizard.access_dm") } else { tr("agent_wizard.access_group") };
        let id = if dm { "edit-dm" } else { "edit-group" };
        v_flex()
            .gap(space(1.))
            .child(
                div()
                    .text_size(text_caption())
                    .text_color(cx.theme().muted_foreground)
                    .child(label),
            )
            .child(h_flex().gap(space(2.)).flex_wrap().children(
                wizard::ACCESS_MODES.into_iter().map(|mode| {
                    Button::new(ElementId::from(format!("agent-{id}-{mode:?}")))
                        .small()
                        .design_secondary(cx)
                        .selected(chosen == mode)
                        .toggled(chosen == mode)
                        .design_label(tr(wizard::access_key(mode)))
                        .on_click(
                            cx.listener(move |this, _, _, cx| {
                                this.choose_form_access(dm, mode, cx)
                            }),
                        )
                }),
            ))
            .into_any_element()
    }

    fn job_row(&mut self, agent: &str, job: &AgentJob, cx: &mut Context<Self>) -> AnyElement {
        let id = job.id.clone();
        let rows = [
            ("pause", AgentJobAction::Pause, "agents.job.pause"),
            ("resume", AgentJobAction::Resume, "agents.job.resume"),
            ("remove", AgentJobAction::Remove, "agents.job.remove"),
        ];
        h_flex()
            .id(ElementId::from(format!("agent-job-{id}")))
            .gap(space(2.))
            .items_center()
            .child(div().flex_1().min_w_0().text_size(text_caption()).child(tr_args(
                "agents.job.line",
                &[("kind", &tr(job_kind_key(job.kind))), ("id", &id)],
            )))
            .children(rows.into_iter().map(|(part, action, key)| {
                let agent = agent.to_owned();
                let job = job.clone();
                Button::new(ElementId::from(format!("agent-job-{part}-{id}")))
                    .xsmall()
                    .design_secondary(cx)
                    .design_label(tr(key))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.job_action(&agent, &job, action, window, cx)
                    }))
            }))
            .into_any_element()
    }

    /// The features shown as unavailable, with their reasons.
    pub(super) fn render_unavailable(&mut self, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .id("agent-unavailable")
            .gap(space(2.))
            .children(UNAVAILABLE.into_iter().enumerate().map(|(ix, reason)| {
                unavailable_line(format!("agent-unavailable-{ix}"), reason, cx)
            }))
            .into_any_element()
    }
}

impl Form {
    /// The agent whose row the form sits under.
    fn agent(&self) -> &str {
        match self {
            Self::Edit { agent, .. } | Self::Access { agent, .. } | Self::Jobs { agent } => agent,
        }
    }
}
