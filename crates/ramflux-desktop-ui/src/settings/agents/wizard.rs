// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The new-agent wizard (design D33–D38, derived #28, #30–#32), bound to the
//! core's [`AgentDraft`]. Nothing is defaulted: a provider, a model, the
//! tools, the limits and an avatar stay unset until the owner picks them,
//! and Create stays disabled until the draft is valid.
//!
//! 1. Basics: name, avatar preset (glyph and tone), responsibility.
//! 2. Runtime: the PRX status on this computer, the provider and the model
//!    (the runtime is installed from the runtime panel above the list).
//! 3. Capabilities: all or no tools, the limits; skills and per-tool
//!    approval are shown as unavailable.
//! 4. Conversations: who may message the agent and who may invite it.
//! 5. Review, then Create. The creation is approved on the owner's phone:
//!    the result follows the request's state (D38) until the owner closes
//!    the wizard.

use std::collections::BTreeSet;

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, Selectable as _, Sizable as _, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Context, ElementId, Entity, Role, SharedString, Window, div};
use ramflux_desktop_core::projection::wizard::{AgentDraft, DraftError, FIELD_LIMIT, WizardStep};
use ramflux_desktop_core::{
    AccountId, AgentAccess, AgentAccessMode, AgentCreationState, AgentCustodyProblem, AgentTools,
    Intent, IntentKind, Store, methods,
};
use ramflux_desktop_navigation::direction::h_flex;
use ramflux_desktop_navigation::palette;
use ramflux_desktop_navigation::scale::{space, text_caption, text_secondary, text_subheading};
use ramflux_desktop_sdk::translate::agents::refusal_key;

use super::{AgentsPanel, muted};
use crate::i18n::{self, tr, tr_args};
use crate::style::DesignButton as _;

/// The glyphs an avatar preset can name.
pub const GLYPHS: [&str; 6] = ["spark", "leaf", "star", "bolt", "moon", "book"];
/// The tones an avatar preset can name.
pub const TONES: [&str; 4] = ["teal", "amber", "rose", "indigo"];
/// The autonomy the runtime applies (the only one it publishes).
const AUTONOMY: &str = "full";

/// The inputs of the wizard's typed fields.
pub struct Inputs {
    pub name: Entity<InputState>,
    pub responsibility: Entity<InputState>,
    pub provider: Entity<InputState>,
    pub model: Entity<InputState>,
    pub turns: Entity<InputState>,
    pub cost: Entity<InputState>,
}

impl Inputs {
    /// The inputs, empty, with their placeholders.
    pub fn new(window: &mut Window, cx: &mut Context<AgentsPanel>) -> Self {
        let field = |key: &'static str, window: &mut Window, cx: &mut Context<AgentsPanel>| {
            crate::i18n::localized_input(key, window, cx, |input| input)
        };
        Self {
            name: field("agent_wizard.name_placeholder", window, cx),
            responsibility: field("agent_wizard.responsibility_placeholder", window, cx),
            provider: field("agent_wizard.provider_placeholder", window, cx),
            model: field("agent_wizard.model_placeholder", window, cx),
            turns: field("agent_wizard.turns_placeholder", window, cx),
            cost: field("agent_wizard.cost_placeholder", window, cx),
        }
    }
}

/// The record of the creation this wizard sent: the request the daemon
/// gave, the first failure of this request, and what was known before.
pub struct Sent {
    /// The creation requests that existed before the send.
    before: BTreeSet<String>,
    /// The failed effects counted before the send.
    failures: u64,
    /// The request this send created, once the store shows it.
    pub request_id: Option<String>,
    /// The translated message of the request's failure, if it failed.
    pub failure: Option<String>,
}

impl Sent {
    /// Starts watching a send made against `store`.
    pub fn new(store: &Store) -> Self {
        Self {
            before: store.agent_creations().map(|c| c.request_id.clone()).collect(),
            failures: store.failures_total(),
            request_id: None,
            failure: None,
        }
    }

    /// Takes the request and the failure of this send from `store`.
    pub fn observe(&mut self, store: &Store) {
        if self.request_id.is_none() {
            self.request_id = store
                .agent_creations()
                .map(|creation| creation.request_id.clone())
                .find(|id| !self.before.contains(id));
        }
        if self.failure.is_none() {
            self.failure = store
                .failures_since(self.failures)
                .iter()
                .find(|failure| failure.method == methods::AGENT_CREATE)
                .map(|failure| failure.message.clone());
        }
    }
}

/// The open wizard: the draft, where the owner is and what was sent.
pub struct Wizard {
    /// Current step.
    pub step: WizardStep,
    /// The draft as last saved from the inputs of the steps passed.
    pub draft: AgentDraft,
    /// The avatar glyph chosen.
    pub glyph: Option<&'static str>,
    /// The avatar tone chosen.
    pub tone: Option<&'static str>,
    /// Why the last step or Create did not proceed.
    pub refusal: Option<DraftError>,
    /// The creation sent, while it is watched.
    pub sent: Option<Sent>,
}

impl Wizard {
    /// A fresh wizard for `owner`: nothing chosen.
    pub fn new(owner: AccountId) -> Self {
        Self {
            step: WizardStep::Basics,
            draft: AgentDraft::new(owner),
            glyph: None,
            tone: None,
            refusal: None,
            sent: None,
        }
    }

    /// The draft as the inputs and the choices make it now.
    fn current(&self, inputs: &Inputs, cx: &App) -> AgentDraft {
        let mut draft = self.draft.clone();
        draft.persona.name = read(&inputs.name, cx);
        draft.persona.responsibility = read(&inputs.responsibility, cx);
        draft.persona.avatar = preset(self.glyph, self.tone).unwrap_or_default();
        draft.provider = read(&inputs.provider, cx);
        draft.model = read(&inputs.model, cx);
        draft.max_concurrent_turns = read(&inputs.turns, cx).trim().parse().ok().filter(|n| *n > 0);
        draft.max_cost_per_day_cents = read(&inputs.cost, cx).trim().parse().ok();
        draft
    }
}

fn read(input: &Entity<InputState>, cx: &App) -> String {
    input.read(cx).value().to_string()
}

/// The avatar preset token `preset:<glyph>:<tone>`, once both are chosen.
pub fn preset(glyph: Option<&str>, tone: Option<&str>) -> Option<String> {
    match (glyph, tone) {
        (Some(glyph), Some(tone)) => Some(format!("preset:{glyph}:{tone}")),
        _ => None,
    }
}

/// The glyph and tone of an avatar preset token, if it is one.
pub fn preset_parts(avatar: &str) -> Option<(&str, &str)> {
    let rest = avatar.strip_prefix("preset:")?;
    let (glyph, tone) = rest.split_once(':')?;
    (!glyph.is_empty() && !tone.is_empty()).then_some((glyph, tone))
}

/// Whether a text field is one the draft accepts.
fn valid_text(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= FIELD_LIMIT && !value.chars().any(char::is_control)
}

/// The fields a step must have before the owner moves on.
fn step_error(draft: &AgentDraft, step: WizardStep) -> Option<DraftError> {
    let missing = |ok: bool, field: &'static str| (!ok).then_some(DraftError::Field(field));
    match step {
        WizardStep::Basics => missing(valid_text(&draft.persona.name), "name")
            .or_else(|| missing(valid_text(&draft.persona.avatar), "avatar"))
            .or_else(|| missing(valid_text(&draft.persona.responsibility), "responsibility")),
        WizardStep::Runtime => missing(valid_text(&draft.provider), "provider")
            .or_else(|| missing(valid_text(&draft.model), "model")),
        WizardStep::Capabilities => missing(draft.tools.is_some(), "tools")
            .or_else(|| missing(draft.max_concurrent_turns.is_some(), "max_concurrent_turns"))
            .or_else(|| missing(draft.max_cost_per_day_cents.is_some(), "max_cost_per_day_cents")),
        WizardStep::Conversations => (!draft.access.is_valid()).then_some(DraftError::Access),
        WizardStep::Review => None,
    }
}

/// The label key of a draft field, for the refusal text.
fn field_key(field: &str) -> &'static str {
    match field {
        "name" => "agent_wizard.field.name",
        "avatar" => "agent_wizard.field.avatar",
        "responsibility" => "agent_wizard.field.responsibility",
        "provider" => "agent_wizard.field.provider",
        "model" => "agent_wizard.field.model",
        "tools" => "agent_wizard.field.tools",
        "max_concurrent_turns" => "agent_wizard.field.turns",
        "max_cost_per_day_cents" => "agent_wizard.field.cost",
        _ => "agent_wizard.field.other",
    }
}

/// What a refusal says.
fn refusal_text(error: DraftError) -> SharedString {
    match error {
        DraftError::Custody(problem) => i18n::agent_custody_problem(Some(problem)),
        DraftError::Field(field) => {
            tr_args("agent_wizard.refusal.field", &[("field", &tr(field_key(field)))])
        }
        DraftError::Access => tr("agent_wizard.refusal.access"),
    }
}

/// The step's label key.
const fn step_key(step: WizardStep) -> &'static str {
    match step {
        WizardStep::Basics => "agent_wizard.step.basics",
        WizardStep::Runtime => "agent_wizard.step.runtime",
        WizardStep::Capabilities => "agent_wizard.step.capabilities",
        WizardStep::Conversations => "agent_wizard.step.conversations",
        WizardStep::Review => "agent_wizard.step.review",
    }
}

fn step_index(step: WizardStep) -> usize {
    WizardStep::ALL.iter().position(|each| *each == step).unwrap_or(0)
}

fn step_offset(step: WizardStep, by: isize) -> WizardStep {
    let next = step_index(step).saturating_add_signed(by).min(WizardStep::ALL.len() - 1);
    WizardStep::ALL.get(next).copied().unwrap_or(step)
}

/// The label of a glyph or a tone in the pickers.
pub(super) fn choice_key(prefix: &'static str, value: &str) -> &'static str {
    match (prefix, value) {
        ("glyph", "spark") => "agent.avatar.glyph.spark",
        ("glyph", "leaf") => "agent.avatar.glyph.leaf",
        ("glyph", "star") => "agent.avatar.glyph.star",
        ("glyph", "bolt") => "agent.avatar.glyph.bolt",
        ("glyph", "moon") => "agent.avatar.glyph.moon",
        ("glyph", _) => "agent.avatar.glyph.book",
        (_, "teal") => "agent.avatar.tone.teal",
        (_, "amber") => "agent.avatar.tone.amber",
        (_, "rose") => "agent.avatar.tone.rose",
        _ => "agent.avatar.tone.indigo",
    }
}

pub(super) fn access_key(mode: AgentAccessMode) -> &'static str {
    match mode {
        AgentAccessMode::Owner => "agent.access.mode.owner",
        AgentAccessMode::Contacts => "agent.access.mode.contacts",
        AgentAccessMode::Allowlist => "agent.access.mode.allowlist",
        AgentAccessMode::Open => "agent.access.mode.open",
    }
}

/// The modes an owner can choose for a new agent's access.
pub const ACCESS_MODES: [AgentAccessMode; 3] =
    [AgentAccessMode::Owner, AgentAccessMode::Contacts, AgentAccessMode::Open];

impl AgentsPanel {
    /// The account that owns the agent a wizard creates: the open account.
    pub(super) fn owner(&self, cx: &App) -> Option<AccountId> {
        self.store.read(cx).active_account().cloned()
    }

    /// Opens the wizard for the open account.
    pub fn begin_wizard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(owner) = self.owner(cx) else { return };
        self.wizard = Some(Wizard::new(owner));
        for input in [
            &self.wizard_inputs.name,
            &self.wizard_inputs.responsibility,
            &self.wizard_inputs.turns,
            &self.wizard_inputs.cost,
        ] {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
        cx.notify();
    }

    /// Closes the wizard; the draft is discarded only here, by the owner.
    pub fn close_wizard(&mut self, cx: &mut Context<Self>) {
        self.wizard = None;
        cx.notify();
    }

    /// Moves `by` steps. Leaving a step needs the fields it asks for.
    pub fn wizard_step(&mut self, by: isize, cx: &mut Context<Self>) {
        let Some(wizard) = self.wizard.as_ref() else { return };
        let draft = wizard.current(&self.wizard_inputs, cx);
        let refusal = (by > 0).then(|| step_error(&draft, wizard.step)).flatten();
        if let Some(wizard) = self.wizard.as_mut() {
            wizard.draft = draft;
            wizard.refusal = refusal;
            if refusal.is_none() {
                wizard.step = step_offset(wizard.step, by);
            }
        }
        cx.notify();
    }

    /// Chooses an avatar glyph or tone (`glyph`: true for the glyph).
    pub fn choose_avatar(&mut self, glyph: bool, value: &'static str, cx: &mut Context<Self>) {
        if let Some(wizard) = self.wizard.as_mut() {
            if glyph {
                wizard.glyph = Some(value);
            } else {
                wizard.tone = Some(value);
            }
        }
        cx.notify();
    }

    /// Chooses the tools: every tool or none.
    pub fn choose_tools(&mut self, tools: AgentTools, cx: &mut Context<Self>) {
        if let Some(wizard) = self.wizard.as_mut() {
            wizard.draft.tools = Some(tools);
        }
        cx.notify();
    }

    /// Chooses who may message the agent (`dm`) or invite it to a group.
    pub fn choose_access(&mut self, dm: bool, mode: AgentAccessMode, cx: &mut Context<Self>) {
        if let Some(wizard) = self.wizard.as_mut() {
            let access: &mut AgentAccess = &mut wizard.draft.access;
            if dm {
                access.dm = mode;
            } else {
                access.group_inviters = mode;
            }
        }
        cx.notify();
    }

    /// Create: asks the core for the request of the draft. The owner's
    /// phone approves it; the result is followed from the store.
    pub fn create_from_wizard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(wizard) = self.wizard.as_ref() else { return };
        let draft = wizard.current(&self.wizard_inputs, cx);
        let request = match draft.request(self.store.read(cx).entry_store()) {
            Ok(request) => request,
            Err(error) => {
                if let Some(wizard) = self.wizard.as_mut() {
                    wizard.draft = draft;
                    wizard.refusal = Some(error);
                }
                cx.notify();
                return;
            }
        };
        let owner = draft.owner.clone();
        let sent = Sent::new(self.store.read(cx).store());
        let kind = IntentKind::AgentCreate { owner_id: owner, request };
        let result = self.root.update(cx, |root, cx| root.submit(Intent::human(kind), window, cx));
        let sending = result.starts_with("submitted") || result == "confirmation required";
        if let Some(wizard) = self.wizard.as_mut() {
            wizard.draft = draft;
            wizard.refusal = None;
            if sending {
                wizard.sent = Some(sent);
            } else {
                wizard.refusal = Some(DraftError::Custody(AgentCustodyProblem::Unknown));
            }
        }
        self.ensure_poll(cx);
        cx.notify();
    }

    /// Goes back to the review of a creation that did not succeed, with the
    /// draft as it was.
    pub fn review_again(&mut self, cx: &mut Context<Self>) {
        if let Some(wizard) = self.wizard.as_mut() {
            wizard.sent = None;
            wizard.step = WizardStep::Review;
        }
        cx.notify();
    }

    /// The wizard's page for the current step.
    pub(super) fn render_wizard(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(wizard) = self.wizard.as_ref() else {
            return div().into_any_element();
        };
        let step = wizard.step;
        let sent = wizard.sent.is_some();
        let refusal = wizard.refusal.map(refusal_text);
        let ready = wizard
            .current(&self.wizard_inputs, cx)
            .request(self.store.read(cx).entry_store())
            .is_ok();
        let theme = palette(cx);
        let steps = h_flex()
            .id("agent-wizard-steps")
            .test_support()
            .role(Role::List)
            .aria_label(tr("agent_wizard.steps"))
            .flex_wrap()
            .gap(space(2.))
            .children(WizardStep::ALL.into_iter().map(|each| {
                let current = each == step;
                div()
                    .id(ElementId::from(format!("agent-wizard-step-{}", step_index(each) + 1)))
                    .role(Role::ListItem)
                    .aria_selected(current)
                    .text_size(text_caption())
                    .px(space(2.))
                    .py(space(1.))
                    .rounded(cx.theme().radius)
                    .border_1()
                    .border_color(if current { theme.primary } else { theme.line })
                    .text_color(if current {
                        cx.theme().foreground
                    } else {
                        cx.theme().muted_foreground
                    })
                    .child(tr_args(
                        "agent_wizard.step_label",
                        &[
                            ("number", &(step_index(each) + 1).to_string()),
                            ("name", &tr(step_key(each))),
                        ],
                    ))
            }));
        let body = if sent {
            self.render_sent(cx)
        } else {
            match step {
                WizardStep::Basics => self.basics(cx),
                WizardStep::Runtime => self.runtime_step(cx),
                WizardStep::Capabilities => self.capabilities(cx),
                WizardStep::Conversations => self.conversations(cx),
                WizardStep::Review => self.review(cx),
            }
        };
        let first = step == WizardStep::Basics;
        let last = step == WizardStep::Review;
        let nav = h_flex()
            .id("agent-wizard-nav")
            .test_support()
            .gap(space(2.))
            .child(
                Button::new("agent-wizard-cancel")
                    .small()
                    .ghost()
                    .label(tr(if sent { "agent_wizard.close" } else { "common.cancel" }))
                    .on_click(cx.listener(|this, _, _, cx| this.close_wizard(cx))),
            )
            .child(div().flex_1())
            .children((!sent && !first).then(|| {
                Button::new("agent-wizard-back")
                    .small()
                    .design_secondary(cx)
                    .design_label(tr("agent_wizard.back"))
                    .on_click(cx.listener(|this, _, _, cx| this.wizard_step(-1, cx)))
            }))
            .children((!sent && !last).then(|| {
                Button::new("agent-wizard-next")
                    .small()
                    .design_primary(cx)
                    .design_label(tr("agent_wizard.next"))
                    .on_click(cx.listener(|this, _, _, cx| this.wizard_step(1, cx)))
            }))
            .children((!sent && last).then(|| {
                Button::new("agent-wizard-create")
                    .small()
                    .design_primary(cx)
                    .design_label(tr("agent_wizard.create"))
                    .design_disabled(!ready, cx)
                    .on_click(
                        cx.listener(|this, _, window, cx| this.create_from_wizard(window, cx)),
                    )
            }));
        v_flex()
            .id("agent-wizard")
            .test_support()
            .role(Role::Group)
            .aria_label(tr("agent_wizard.title"))
            .gap(space(3.))
            .min_w_0()
            .child(div().text_size(text_subheading()).child(tr("agent_wizard.title")))
            .child(steps)
            .child(div().font_weight(gpui_kit::FontWeight::MEDIUM).child(tr(step_key(step))))
            .child(body)
            .children(refusal.map(|text| {
                div()
                    .id("agent-wizard-refusal")
                    .test_support()
                    .role(Role::Alert)
                    .aria_label(text.clone())
                    .whitespace_normal()
                    .child(text)
            }))
            .child(nav)
            .into_any_element()
    }

    fn basics(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let glyph = self.wizard.as_ref().and_then(|w| w.glyph);
        let tone = self.wizard.as_ref().and_then(|w| w.tone);
        v_flex()
            .gap(space(2.))
            .child(muted(tr("agent_wizard.basics_help"), cx))
            .child(
                Input::new(&self.wizard_inputs.name)
                    .id("agent-wizard-name")
                    .aria_label(tr("agent_wizard.name")),
            )
            .child(
                Input::new(&self.wizard_inputs.responsibility)
                    .id("agent-wizard-responsibility")
                    .aria_label(tr("agent_wizard.responsibility")),
            )
            .child(muted(tr("agent_wizard.avatar_help"), cx))
            .child(self.choice_row("glyph", &GLYPHS, glyph, cx))
            .child(self.choice_row("tone", &TONES, tone, cx))
            .into_any_element()
    }

    /// One row of the avatar pickers.
    fn choice_row(
        &mut self,
        prefix: &'static str,
        values: &'static [&'static str],
        chosen: Option<&'static str>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let glyph = prefix == "glyph";
        h_flex()
            .id(ElementId::from(format!("agent-wizard-{prefix}s")))
            .role(Role::List)
            .gap(space(2.))
            .flex_wrap()
            .children(values.iter().map(|value| {
                let value: &'static str = value;
                let selected = chosen == Some(value);
                Button::new(ElementId::from(format!("agent-wizard-{prefix}-{value}")))
                    .small()
                    .design_secondary(cx)
                    .selected(selected)
                    .toggled(selected)
                    .design_label(tr(choice_key(prefix, value)))
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.choose_avatar(glyph, value, cx)),
                    )
            }))
            .into_any_element()
    }

    fn runtime_step(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let status = super::runtime::status_line(self.store.read(cx).store());
        v_flex()
            .gap(space(2.))
            .child(muted(tr("agent_wizard.runtime_help"), cx))
            .child(muted(status, cx))
            .child(
                Input::new(&self.wizard_inputs.provider)
                    .id("agent-wizard-provider")
                    .aria_label(tr("agent_wizard.provider")),
            )
            .child(
                Input::new(&self.wizard_inputs.model)
                    .id("agent-wizard-model")
                    .aria_label(tr("agent_wizard.model")),
            )
            .child(muted(tr("agent_wizard.provider_free_text"), cx))
            .into_any_element()
    }

    fn capabilities(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let tools = self.wizard.as_ref().and_then(|w| w.draft.tools);
        v_flex()
            .gap(space(2.))
            .child(muted(tr("agent_wizard.capabilities_help"), cx))
            .child(
                h_flex()
                    .gap(space(2.))
                    .child(
                        Button::new("agent-wizard-tools-all")
                            .small()
                            .design_secondary(cx)
                            .selected(tools == Some(AgentTools::All))
                            .toggled(tools == Some(AgentTools::All))
                            .design_label(tr("agent_wizard.tools_all"))
                            .on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.choose_tools(AgentTools::All, cx)
                                }),
                            ),
                    )
                    .child(
                        Button::new("agent-wizard-tools-none")
                            .small()
                            .design_secondary(cx)
                            .selected(tools == Some(AgentTools::None))
                            .toggled(tools == Some(AgentTools::None))
                            .design_label(tr("agent_wizard.tools_none"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.choose_tools(AgentTools::None, cx)
                            })),
                    ),
            )
            .child(
                Input::new(&self.wizard_inputs.turns)
                    .id("agent-wizard-turns")
                    .aria_label(tr("agent_wizard.turns")),
            )
            .child(
                Input::new(&self.wizard_inputs.cost)
                    .id("agent-wizard-cost")
                    .aria_label(tr("agent_wizard.cost")),
            )
            .child(super::rows::unavailable_line(
                "agent-wizard-skills",
                ramflux_desktop_core::AgentUnavailable::Skills,
                cx,
            ))
            .child(super::rows::unavailable_line(
                "agent-wizard-autonomy",
                ramflux_desktop_core::AgentUnavailable::AutonomyHonoured,
                cx,
            ))
            .child(muted(tr_args("agent_wizard.autonomy_fixed", &[("autonomy", AUTONOMY)]), cx))
            .child(super::rows::unavailable_line(
                "agent-wizard-per-tool",
                ramflux_desktop_core::AgentUnavailable::PerToolApproval,
                cx,
            ))
            .into_any_element()
    }

    fn conversations(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let access = self
            .wizard
            .as_ref()
            .map(|w| w.draft.access.clone())
            .unwrap_or_else(AgentAccess::owner_only);
        v_flex()
            .gap(space(2.))
            .child(muted(tr("agent_wizard.conversations_help"), cx))
            .child(self.access_row(true, access.dm, cx))
            .child(self.access_row(false, access.group_inviters, cx))
            .child(super::rows::unavailable_line(
                "agent-wizard-sub-agent",
                ramflux_desktop_core::AgentUnavailable::SubAgent,
                cx,
            ))
            .into_any_element()
    }

    /// One access choice: who may message (`dm`) or invite the agent.
    pub(super) fn access_row(
        &mut self,
        dm: bool,
        chosen: AgentAccessMode,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let label = if dm { tr("agent_wizard.access_dm") } else { tr("agent_wizard.access_group") };
        let id = if dm { "dm" } else { "group" };
        v_flex()
            .gap(space(1.))
            .child(
                div()
                    .text_size(text_caption())
                    .text_color(cx.theme().muted_foreground)
                    .child(label),
            )
            .child(h_flex().gap(space(2.)).flex_wrap().children(ACCESS_MODES.into_iter().map(
                |mode| {
                    Button::new(ElementId::from(format!("agent-wizard-access-{id}-{mode:?}")))
                        .small()
                        .design_secondary(cx)
                        .selected(chosen == mode)
                        .toggled(chosen == mode)
                        .design_label(tr(access_key(mode)))
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.choose_access(dm, mode, cx)),
                        )
                },
            )))
            .into_any_element()
    }

    fn review(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(wizard) = self.wizard.as_ref() else { return div().into_any_element() };
        let draft = wizard.current(&self.wizard_inputs, cx);
        let tools = match draft.tools {
            Some(AgentTools::All) => tr("agent_wizard.tools_all"),
            Some(AgentTools::None) => tr("agent_wizard.tools_none"),
            None => tr("agent_wizard.none"),
        };
        let limit = |value: Option<u64>| {
            value.map_or_else(|| tr("agent_wizard.none"), |v| v.to_string().into())
        };
        let facts: [(&str, SharedString); 8] = [
            ("agent-review-name", tr("agent_wizard.name")),
            (
                "agent-review-avatar",
                preset(wizard.glyph, wizard.tone)
                    .map_or_else(|| tr("agent_wizard.none"), SharedString::from),
            ),
            (
                "agent-review-provider",
                SharedString::from(format!("{} / {}", draft.provider, draft.model)),
            ),
            ("agent-review-tools", tools),
            ("agent-review-turns", limit(draft.max_concurrent_turns.map(u64::from))),
            ("agent-review-cost", limit(draft.max_cost_per_day_cents)),
            ("agent-review-dm", tr(access_key(draft.access.dm))),
            ("agent-review-group", tr(access_key(draft.access.group_inviters))),
        ];
        let name = draft.persona.name.trim().to_owned();
        v_flex()
            .id("agent-wizard-review")
            .test_support()
            .gap(space(2.))
            .child(
                div()
                    .text_size(text_secondary())
                    .font_weight(gpui_kit::FontWeight::MEDIUM)
                    .child(name),
            )
            .children(facts.into_iter().map(|(id, value)| {
                div()
                    .id(id)
                    .test_support()
                    .text_size(text_secondary())
                    .whitespace_normal()
                    .child(value)
            }))
            .child(muted(tr("agent_wizard.review_help"), cx))
            .into_any_element()
    }

    /// The result of the creation sent (D38, derived #28): waiting for the
    /// phone, confirming, created, or why it did not succeed.
    fn render_sent(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(sent) = self.wizard.as_ref().and_then(|w| w.sent.as_ref()) else {
            return div().into_any_element();
        };
        let failure = sent.failure.clone();
        let creation = sent
            .request_id
            .as_ref()
            .and_then(|id| self.store.read(cx).store().agent_creation(id).cloned());
        let (text, spinning) = match (&failure, creation.as_ref().map(|c| (&c.state, c.expires_at)))
        {
            (Some(key), _) => (tr(key), false),
            (None, None) => (tr("agent_wizard.sending"), true),
            (None, Some((state, expires_at))) => match state {
                AgentCreationState::Draft | AgentCreationState::PendingPhone => {
                    let left = super::seconds_left(expires_at).unwrap_or_default().as_secs();
                    let clock = format!("{}:{:02}", left / 60, left % 60);
                    (tr_args("agent_wizard.pending_phone", &[("time", &clock)]), false)
                }
                AgentCreationState::Confirming => (tr("agent_wizard.confirming"), true),
                AgentCreationState::Created => (tr("agent_wizard.created"), false),
                AgentCreationState::Denied => (tr("agent_wizard.denied"), false),
                AgentCreationState::Expired => (tr("agent_wizard.expired"), false),
                AgentCreationState::Cancelled => (tr("agent_wizard.cancelled"), false),
                AgentCreationState::Failed(refusal) => (tr(refusal_key(refusal)), false),
            },
        };
        let ended = creation.as_ref().is_some_and(|c| c.state.is_terminal());
        let created =
            matches!(creation.as_ref().map(|c| &c.state), Some(AgentCreationState::Created));
        let retry = failure.is_none() && ended && !created;
        v_flex()
            .id("agent-wizard-result")
            .test_support()
            .role(Role::Status)
            .aria_label(text.clone())
            .gap(space(2.))
            .child(
                h_flex()
                    .gap(space(2.))
                    .items_center()
                    .text_size(text_secondary())
                    .children(spinning.then(|| Spinner::new().small()))
                    .child(text),
            )
            .children(retry.then(|| {
                Button::new("agent-wizard-review-again")
                    .small()
                    .design_secondary(cx)
                    .design_label(tr("agent_wizard.review_again"))
                    .on_click(cx.listener(|this, _, _, cx| this.review_again(cx)))
            }))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_preset_names_a_glyph_and_a_tone_only_when_both_are_chosen() {
        assert_eq!(preset(Some("spark"), Some("teal")).as_deref(), Some("preset:spark:teal"));
        assert_eq!(preset(Some("spark"), None), None);
        assert_eq!(preset_parts("preset:spark:teal"), Some(("spark", "teal")));
        assert_eq!(preset_parts("data:image/png;base64,AA"), None);
    }

    #[test]
    fn a_step_needs_its_own_fields_only() {
        let draft = AgentDraft::new(AccountId::new("owner"));
        assert!(matches!(step_error(&draft, WizardStep::Basics), Some(DraftError::Field("name"))));
        assert!(step_error(&draft, WizardStep::Review).is_none());
        assert!(step_error(&draft, WizardStep::Conversations).is_none());
    }

    #[test]
    fn the_step_order_is_the_core_order() {
        assert_eq!(step_offset(WizardStep::Basics, 1), WizardStep::Runtime);
        assert_eq!(step_offset(WizardStep::Review, 1), WizardStep::Review);
        assert_eq!(step_offset(WizardStep::Basics, -1), WizardStep::Basics);
    }
}
