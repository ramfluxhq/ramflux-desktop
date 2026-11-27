// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The configuration of an agent after creation: its model (chosen from the
//! PRX catalog), its autonomy and its prompt, and pausing and resuming it.
//!
//! A change is sent with the revision the screen was rendered from (the
//! per-agent one `agent.get` shows). The form reads the agent again as it
//! opens, so that revision is the daemon's current one. A change made
//! elsewhere is refused as `revision_conflict`: the agent is read again and
//! the owner reviews it. A change PRX holds for confirmation
//! (`confirmation_required`) shows a confirm button; only its answer resends
//! the change with `confirm_high_risk`.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::Selectable as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::Input;
use gpui_kit::component::v_flex;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Context, ElementId, Role, SharedString, Window, div};
use ramflux_desktop_core::{AgentChange, AgentRecord, Intent, IntentKind, PromptChange};
use ramflux_desktop_navigation::direction::h_flex;
use ramflux_desktop_navigation::scale::{space, text_caption, text_secondary};

use super::{AgentsPanel, Form, muted};
use crate::i18n::tr;
use crate::style::DesignButton as _;

/// The autonomy levels PRX knows, with their label keys.
const AUTONOMY: [(&str, &str); 3] = [
    ("full", "agents.settings.autonomy_full"),
    ("supervised", "agents.settings.autonomy_supervised"),
    ("readonly", "agents.settings.autonomy_readonly"),
];

/// The failure message that asks the owner to confirm a change.
const CONFIRMATION_KEY: &str = "backend.agent.confirmation_required";
/// The failure message of a change made on a revision that moved on.
const CONFLICT_KEY: &str = "backend.agent.revision_conflict";

impl AgentsPanel {
    /// Reads without a window: the catalog and one agent's record.
    fn read(&self, kind: IntentKind, cx: &mut Context<Self>) {
        self.store.update(cx, |store, cx| store.dispatch(Intent::human(kind), cx));
    }

    /// Opens the settings form of `agent`: the catalog and the agent are read
    /// first, so the choices and the revision are the daemon's current ones.
    pub(super) fn open_settings(
        &mut self,
        agent: &AgentRecord,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let settings = agent.settings.clone().unwrap_or_default();
        let prompt = settings.prompt.clone().unwrap_or_default();
        self.prompt_input.update(cx, |input, cx| input.set_value(prompt, window, cx));
        self.settings_sent = false;
        self.read(IntentKind::AgentCatalogRefresh, cx);
        self.read(IntentKind::AgentRead { agent: agent.agent_anchor.clone() }, cx);
        self.open_form(
            Some(Form::Settings {
                agent: agent.agent_anchor.clone(),
                model: Some((settings.provider, settings.model)),
                autonomy: Some(settings.autonomy),
                reset_prompt: false,
            }),
            cx,
        );
    }

    /// Chooses a model of the catalog in the open settings form.
    pub(super) fn choose_settings_model(
        &mut self,
        provider: &str,
        model: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(Form::Settings { model: chosen, .. }) = self.form.as_mut() {
            *chosen = Some((provider.to_owned(), model.to_owned()));
        }
        cx.notify();
    }

    /// Sends the settings form: only what differs from the configuration the
    /// screen shows, with that configuration's revision.
    pub(super) fn save_settings(
        &mut self,
        confirm_high_risk: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(Form::Settings { agent, model, autonomy, reset_prompt }) = self.form.as_ref()
        else {
            return;
        };
        let Some(owner_id) = self.owner(cx) else { return };
        let Some(record) = self.state(cx).agent(agent).cloned() else { return };
        let held = record.settings.clone().unwrap_or_default();
        let mut change = AgentChange::default();
        if let Some((provider, name)) = model
            && (held.provider != *provider || held.model != *name)
        {
            change.provider = Some(provider.clone());
            change.model = Some(name.clone());
        }
        if let Some(level) = autonomy.as_ref().filter(|level| **level != held.autonomy) {
            change.autonomy = Some(level.clone());
        }
        let typed = self.prompt_input.read(cx).value().to_string();
        change.prompt = if *reset_prompt || (typed.trim().is_empty() && held.prompt.is_some()) {
            PromptChange::Reset
        } else if !typed.trim().is_empty() && Some(&typed) != held.prompt.as_ref() {
            PromptChange::Set(typed)
        } else {
            PromptChange::Keep
        };
        let kind = IntentKind::AgentUpdate {
            owner_id,
            agent: record.agent_anchor,
            revision: held.revision,
            change,
            confirm_high_risk,
        };
        self.settings_sent = true;
        self.submit(kind, window, cx);
        cx.notify();
    }

    /// Pauses or resumes `agent`, on the revision the row was drawn from.
    pub(super) fn toggle_pause(
        &mut self,
        agent: &AgentRecord,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(owner_id) = self.owner(cx) else { return };
        let settings = agent.settings.clone().unwrap_or_default();
        let kind = IntentKind::AgentSwitch {
            owner_id,
            agent: agent.agent_anchor.clone(),
            revision: settings.revision,
            // A paused agent (not enabled) resumes; any other pauses.
            enabled: agent.settings.as_ref().is_some_and(|s| !s.enabled),
        };
        self.submit(kind, window, cx);
    }

    /// After a change made on an older revision, the agent is read again so
    /// the screen shows what the daemon holds now.
    pub(super) fn follow_settings(&mut self, cx: &mut Context<Self>) {
        let conflict = self.settings_sent && self.last_failure(cx).as_deref() == Some(CONFLICT_KEY);
        if !conflict {
            return;
        }
        self.settings_sent = false;
        if let Some(Form::Settings { agent, .. }) = self.form.as_ref() {
            let agent = agent.clone();
            self.read(IntentKind::AgentRead { agent }, cx);
        }
    }

    /// The settings form under an agent's row.
    pub(super) fn settings_form(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(Form::Settings { model, autonomy, reset_prompt, .. }) = self.form.as_ref() else {
            return div().into_any_element();
        };
        let (chosen, level, reset) = (
            model.clone().unwrap_or_default(),
            autonomy.clone().unwrap_or_default(),
            *reset_prompt,
        );
        let catalog = self.state(cx).agent_catalog().map(|catalog| catalog.models.clone());
        let models = match catalog {
            None => muted(tr("agents.catalog.loading"), cx).into_any_element(),
            Some(models) => v_flex()
                .id("agent-settings-models")
                .role(Role::List)
                .gap(space(2.))
                .children(models.into_iter().enumerate().map(|(ix, model)| {
                    self.model_button(
                        format!("agent-settings-model-{ix}"),
                        &model,
                        &chosen,
                        Self::choose_settings_model,
                        cx,
                    )
                }))
                .into_any_element(),
        };
        let levels =
            h_flex().id("agent-settings-autonomy").role(Role::List).gap(space(2.)).children(
                AUTONOMY.iter().map(|(value, key)| {
                    let selected = level == *value;
                    let value = (*value).to_owned();
                    Button::new(ElementId::from(format!("agent-settings-autonomy-{value}")))
                        .small()
                        .design_secondary(cx)
                        .selected(selected)
                        .toggled(selected)
                        .design_label(tr(key))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(Form::Settings { autonomy, .. }) = this.form.as_mut() {
                                *autonomy = Some(value.clone());
                            }
                            cx.notify();
                        }))
                }),
            );
        let needs_confirm =
            self.settings_sent && self.last_failure(cx).as_deref() == Some(CONFIRMATION_KEY);
        let changed_elsewhere = self.last_failure(cx).as_deref() == Some(CONFLICT_KEY);
        v_flex()
            .gap(space(2.))
            .child(muted(tr("agents.settings.model"), cx))
            .child(models)
            .child(muted(tr("agents.settings.autonomy"), cx))
            .child(levels)
            .child(muted(tr("agents.settings.prompt"), cx))
            .child(
                Input::new(&self.prompt_input)
                    .id("agent-settings-prompt")
                    .aria_label(tr("agents.settings.prompt")),
            )
            .child(
                Button::new("agent-settings-prompt-reset")
                    .small()
                    .design_secondary(cx)
                    .selected(reset)
                    .toggled(reset)
                    .design_label(tr("agents.settings.prompt_reset"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(Form::Settings { reset_prompt, .. }) = this.form.as_mut() {
                            *reset_prompt = !*reset_prompt;
                        }
                        cx.notify();
                    })),
            )
            .children(changed_elsewhere.then(|| {
                div()
                    .id("agent-settings-conflict")
                    .role(Role::Alert)
                    .test_support()
                    .text_size(text_caption())
                    .text_color(cx.theme().danger)
                    .child(tr("backend.agent.revision_conflict"))
            }))
            .children(needs_confirm.then(|| {
                v_flex()
                    .id("agent-settings-confirm")
                    .role(Role::Alert)
                    .test_support()
                    .gap(space(1.))
                    .child(
                        div()
                            .text_size(text_secondary())
                            .child(tr("agents.settings.confirm_high_risk")),
                    )
                    .child(
                        Button::new("agent-settings-confirm-send")
                            .small()
                            .design_primary(cx)
                            .design_label(tr("agents.settings.confirm_send"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.save_settings(true, window, cx);
                            })),
                    )
            }))
            .child(
                h_flex()
                    .gap(space(2.))
                    .child(
                        Button::new("agent-settings-save")
                            .small()
                            .design_primary(cx)
                            .design_label(tr("agents.form.save"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.save_settings(false, window, cx);
                            })),
                    )
                    .child(self.close_button(cx)),
            )
            .into_any_element()
    }
}

/// The label of the pause button of an agent: pause, or resume while paused.
pub(super) fn pause_label(agent: &AgentRecord) -> SharedString {
    if agent.settings.as_ref().is_some_and(|settings| !settings.enabled) {
        tr("agents.row.resume")
    } else {
        tr("agents.row.pause")
    }
}
