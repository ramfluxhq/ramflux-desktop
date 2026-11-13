// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The new-Agent wizard (design D33–D38, MOB-D13 style), bound to the
//! core's validated [`AgentDraft`]:
//!
//! 1. Basics: the name.
//! 2. Runtime: a runtime the host offers, or Bind later (the Agent is
//!    created not running).
//! 3. Capabilities: the tools the chosen runtime offers; nothing is on by
//!    default.
//! 4. Conversations: the scopes the chosen runtime offers; nothing is on by
//!    default.
//! 5. Review: every choice, then Create.
//!
//! The draft starts with no runtime and no permissions and only ever holds
//! what the host offered. Create asks the core for the identity intent:
//! an identity-only draft continues into the existing custody ceremony
//! (recovery phrase shown once, D38's result there); a draft with any
//! runtime, tool or scope is refused as unavailable by the core, shown with
//! the typed reason, and kept as it is. Nothing chosen is ever dropped.
//!
//! This version of the local service reports no runtime catalog, so the
//! runtime step shows the typed unavailable reason and offers Bind later.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::Input;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _, h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Context, ElementId, Role, SharedString, Window, div};
use ramflux_desktop_core::projection::wizard::{AgentDraft, DraftError, RuntimeOffer};
use ramflux_desktop_core::{AccountId, IntentKind};
use ramflux_desktop_navigation::generated::catalog;
use ramflux_desktop_navigation::palette;
use ramflux_desktop_sdk::business::ContractGap;

use super::AgentsPanel;
use crate::i18n::{self, tr, tr_args};

/// A wizard step, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WizardStep {
    /// Name.
    Basics,
    /// Runtime binding.
    Runtime,
    /// Tools.
    Capabilities,
    /// Scopes (which conversations may invite the Agent).
    Conversations,
    /// Review and create.
    Review,
}

impl WizardStep {
    /// Every step in order.
    pub const ALL: [Self; 5] =
        [Self::Basics, Self::Runtime, Self::Capabilities, Self::Conversations, Self::Review];

    const fn key(self) -> &'static str {
        match self {
            Self::Basics => "agent_wizard.step.basics",
            Self::Runtime => "agent_wizard.step.runtime",
            Self::Capabilities => "agent_wizard.step.capabilities",
            Self::Conversations => "agent_wizard.step.conversations",
            Self::Review => "agent_wizard.step.review",
        }
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|step| *step == self).unwrap_or(0)
    }

    fn offset(self, by: isize) -> Self {
        let next = self.index().saturating_add_signed(by).min(Self::ALL.len() - 1);
        Self::ALL.get(next).copied().unwrap_or(self)
    }
}

/// The open wizard: the draft and where the owner is.
pub struct Wizard {
    /// Current step.
    pub step: WizardStep,
    /// The draft (owner, name, runtime, tools, scopes).
    pub draft: AgentDraft,
    /// Why the last Create (or Next) did not proceed.
    pub refusal: Option<DraftError>,
}

impl Wizard {
    /// A fresh wizard for `owner`: no runtime, nothing granted.
    pub fn new(owner: AccountId) -> Self {
        Self { step: WizardStep::Basics, draft: AgentDraft::new(owner), refusal: None }
    }
}

/// What a refusal says.
fn refusal_text(error: DraftError) -> SharedString {
    match error {
        DraftError::ConfigurationUnavailable => {
            crate::workspace::sections::directory::unavailable(ContractGap::AgentConfiguration)
        }
        DraftError::Custody(problem) => i18n::agent_custody_problem(Some(problem)),
        DraftError::Field => tr("agent_wizard.refusal.field"),
        DraftError::Limit => tr("agent_wizard.refusal.limit"),
        DraftError::Runtime => tr("agent_wizard.refusal.runtime"),
        DraftError::Permission => tr("agent_wizard.refusal.permission"),
    }
}

impl AgentsPanel {
    /// The runtimes the host offers. This local service reports none yet.
    pub(crate) fn runtime_offers(&self) -> Vec<RuntimeOffer> {
        Vec::new()
    }

    /// Opens the wizard for the open account.
    pub fn begin_wizard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(owner) = self.owner(cx) else { return };
        self.wizard = Some(Wizard::new(owner));
        self.wizard_name.update(cx, |input, cx| input.set_value("", window, cx));
        cx.notify();
    }

    /// Closes the wizard; the draft is discarded only here, by the owner.
    pub fn close_wizard(&mut self, cx: &mut Context<Self>) {
        self.wizard = None;
        cx.notify();
    }

    /// The open wizard.
    pub fn wizard(&self) -> Option<&Wizard> {
        self.wizard.as_ref()
    }

    /// Copies the typed name into the draft.
    fn sync_name(&mut self, cx: &mut Context<Self>) {
        let name = self.wizard_name.read(cx).value().to_string();
        if let Some(wizard) = self.wizard.as_mut() {
            wizard.draft.name = name;
        }
    }

    /// Moves `by` steps; leaving Basics needs a usable name.
    pub fn wizard_step(&mut self, by: isize, cx: &mut Context<Self>) {
        self.sync_name(cx);
        let Some(wizard) = self.wizard.as_mut() else { return };
        if by > 0 && wizard.step == WizardStep::Basics {
            let name = wizard.draft.name.trim();
            if name.is_empty() || name.len() > ramflux_desktop_core::projection::wizard::FIELD_LIMIT
            {
                wizard.refusal = Some(DraftError::Field);
                cx.notify();
                return;
            }
        }
        wizard.refusal = None;
        wizard.step = wizard.step.offset(by);
        cx.notify();
    }

    /// Chooses the runtime `id` (empty: bind later). Tools and scopes that
    /// the new runtime does not offer are cleared, never carried over.
    pub fn choose_runtime(&mut self, id: String, cx: &mut Context<Self>) {
        let offers = self.runtime_offers();
        let Some(wizard) = self.wizard.as_mut() else { return };
        if wizard.draft.runtime != id {
            let offer = offers.iter().find(|offer| offer.id == id);
            wizard.draft.tools.retain(|tool| offer.is_some_and(|o| o.tools.contains(tool)));
            wizard.draft.scopes.retain(|scope| offer.is_some_and(|o| o.scopes.contains(scope)));
            wizard.draft.runtime = id;
        }
        cx.notify();
    }

    fn toggle_tool(&mut self, tool: &str, cx: &mut Context<Self>) {
        if let Some(wizard) = self.wizard.as_mut()
            && !wizard.draft.tools.remove(tool)
        {
            wizard.draft.tools.insert(tool.to_owned());
        }
        cx.notify();
    }

    fn toggle_scope(&mut self, scope: &str, cx: &mut Context<Self>) {
        if let Some(wizard) = self.wizard.as_mut()
            && !wizard.draft.scopes.remove(scope)
        {
            wizard.draft.scopes.insert(scope.to_owned());
        }
        cx.notify();
    }

    /// Create: asks the core for the identity intent against current facts.
    /// An identity-only draft continues into the custody ceremony with its
    /// name; anything the backend cannot carry is refused and kept.
    pub fn create_from_wizard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_name(cx);
        let offers = self.runtime_offers();
        let Some(wizard) = self.wizard.as_ref() else { return };
        let result = wizard.draft.begin_identity(self.store.read(cx).entry_store(), &offers);
        match result {
            Ok(IntentKind::BeginAgentCeremony { label, custodian_id }) => {
                self.wizard = None;
                self.ceremony.update(cx, |ceremony, cx| {
                    ceremony.set_custodian(Some(custodian_id), cx);
                    ceremony.open(window, cx);
                    ceremony
                        .name_input()
                        .update(cx, |input, cx| input.set_value(label, window, cx));
                });
                self.creating = true;
            }
            Ok(_) => {
                if let Some(wizard) = self.wizard.as_mut() {
                    wizard.refusal = Some(DraftError::ConfigurationUnavailable);
                }
            }
            Err(error) => {
                if let Some(wizard) = self.wizard.as_mut() {
                    wizard.refusal = Some(error);
                }
            }
        }
        cx.notify();
    }

    /// The wizard's page for the current step.
    pub(super) fn render_wizard(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(wizard) = self.wizard.as_ref() else {
            return div().into_any_element();
        };
        let step = wizard.step;
        let refusal = wizard.refusal.map(refusal_text);
        let theme = palette(cx);
        let steps = h_flex()
            .id("agent-wizard-steps")
            .test_support()
            .role(Role::List)
            .aria_label(tr("agent_wizard.steps"))
            .flex_wrap()
            .gap_2()
            .children(WizardStep::ALL.into_iter().map(|each| {
                let current = each == step;
                div()
                    .id(ElementId::from(format!("agent-wizard-step-{}", each.index() + 1)))
                    .role(Role::ListItem)
                    .aria_selected(current)
                    .text_xs()
                    .px_2()
                    .py_1()
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
                        &[("number", &(each.index() + 1).to_string()), ("name", &tr(each.key()))],
                    ))
            }));
        let body = match step {
            WizardStep::Basics => self.basics(cx),
            WizardStep::Runtime => self.runtime(cx),
            WizardStep::Capabilities => self.capabilities(cx),
            WizardStep::Conversations => self.conversations(cx),
            WizardStep::Review => self.review(cx),
        };
        let first = step == WizardStep::Basics;
        let last = step == WizardStep::Review;
        let nav = h_flex()
            .id("agent-wizard-nav")
            .test_support()
            .gap_2()
            .child(
                Button::new("agent-wizard-cancel")
                    .small()
                    .ghost()
                    .label(tr("common.cancel"))
                    .on_click(cx.listener(|this, _, _, cx| this.close_wizard(cx))),
            )
            .child(div().flex_1())
            .child(
                Button::new("agent-wizard-back")
                    .small()
                    .outline()
                    .label(tr("agent_wizard.back"))
                    .disabled(first)
                    .on_click(cx.listener(|this, _, _, cx| this.wizard_step(-1, cx))),
            )
            .child(if last {
                Button::new("agent-wizard-create")
                    .small()
                    .primary()
                    .label(tr("agent_wizard.create"))
                    .on_click(
                        cx.listener(|this, _, window, cx| this.create_from_wizard(window, cx)),
                    )
            } else {
                Button::new("agent-wizard-next")
                    .small()
                    .primary()
                    .label(tr("agent_wizard.next"))
                    .on_click(cx.listener(|this, _, _, cx| this.wizard_step(1, cx)))
            });
        v_flex()
            .id("agent-wizard")
            .test_support()
            .role(Role::Group)
            .aria_label(tr("agent_wizard.title"))
            .gap_3()
            .min_w_0()
            .child(div().text_lg().child(tr("agent_wizard.title")))
            .child(steps)
            .child(div().font_weight(gpui_kit::FontWeight::MEDIUM).child(tr(step.key())))
            .child(body)
            .children(refusal.map(|text| {
                div()
                    .id("agent-wizard-refusal")
                    .test_support()
                    .role(Role::Alert)
                    .aria_label(text.clone())
                    .whitespace_normal()
                    .child(catalog::state_block(
                        "agent-wizard-refusal".into(),
                        tr("agent_wizard.refused"),
                        text,
                        catalog::StateKind::Error,
                        div(),
                        theme,
                    ))
            }))
            .child(nav)
            .into_any_element()
    }

    fn basics(&mut self, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .whitespace_normal()
                    .child(tr("agent_wizard.basics_help")),
            )
            .child(
                Input::new(&self.wizard_name)
                    .id("agent-wizard-name")
                    .aria_label(tr("agent_wizard.name")),
            )
            .into_any_element()
    }

    fn runtime(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let offers = self.runtime_offers();
        let chosen = self.wizard.as_ref().map(|w| w.draft.runtime.clone()).unwrap_or_default();
        let theme = palette(cx);
        let mut col = v_flex().gap_2().child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .whitespace_normal()
                .child(tr("agent_wizard.runtime_help")),
        );
        if offers.is_empty() {
            col = col.child(
                div()
                    .id("agent-wizard-no-runtime")
                    .test_support()
                    .role(Role::Status)
                    .whitespace_normal()
                    .child(catalog::state_block(
                        "agent-wizard-no-runtime".into(),
                        tr("agent_wizard.no_runtime"),
                        crate::workspace::sections::directory::unavailable(
                            ContractGap::AgentConfiguration,
                        ),
                        catalog::StateKind::Empty,
                        div(),
                        theme,
                    )),
            );
        }
        for offer in offers {
            let id = offer.id.clone();
            col = col.child(
                Button::new(ElementId::from(format!("agent-wizard-runtime-{}", offer.id)))
                    .small()
                    .outline()
                    .selected(chosen == offer.id)
                    .toggled(chosen == offer.id)
                    .label(SharedString::from(offer.id.clone()))
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.choose_runtime(id.clone(), cx)),
                    ),
            );
        }
        col.child(
            Button::new("agent-wizard-bind-later")
                .small()
                .outline()
                .selected(chosen.is_empty())
                .toggled(chosen.is_empty())
                .label(tr("agent_wizard.bind_later"))
                .on_click(cx.listener(|this, _, _, cx| this.choose_runtime(String::new(), cx))),
        )
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .whitespace_normal()
                .child(tr("agent_wizard.bind_later_help")),
        )
        .into_any_element()
    }

    /// Checkboxes over what the chosen runtime offers (`tools` or scopes).
    fn choices(&mut self, tools: bool, cx: &mut Context<Self>) -> AnyElement {
        let offers = self.runtime_offers();
        let Some(wizard) = self.wizard.as_ref() else { return div().into_any_element() };
        let offer = offers.iter().find(|offer| offer.id == wizard.draft.runtime);
        let offered: Vec<String> = offer
            .map(|o| {
                if tools {
                    o.tools.iter().cloned().collect()
                } else {
                    o.scopes.iter().cloned().collect()
                }
            })
            .unwrap_or_default();
        let chosen = if tools { wizard.draft.tools.clone() } else { wizard.draft.scopes.clone() };
        let help = if tools {
            "agent_wizard.capabilities_help"
        } else {
            "agent_wizard.conversations_help"
        };
        let mut col = v_flex().gap_2().child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .whitespace_normal()
                .child(tr(help)),
        );
        if offered.is_empty() {
            col = col.child(
                div()
                    .id(if tools { "agent-wizard-no-tools" } else { "agent-wizard-no-scopes" })
                    .test_support()
                    .text_sm()
                    .whitespace_normal()
                    .child(tr(if wizard.draft.runtime.is_empty() {
                        "agent_wizard.nothing_unbound"
                    } else {
                        "agent_wizard.nothing_offered"
                    })),
            );
        }
        for item in offered {
            let checked = chosen.contains(&item);
            let key = item.clone();
            let prefix = if tools { "tool" } else { "scope" };
            col = col.child(
                Checkbox::new(ElementId::from(format!("agent-wizard-{prefix}-{item}")))
                    .checked(checked)
                    .label(SharedString::from(item))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if tools {
                            this.toggle_tool(&key, cx);
                        } else {
                            this.toggle_scope(&key, cx);
                        }
                    })),
            );
        }
        col.into_any_element()
    }

    fn capabilities(&mut self, cx: &mut Context<Self>) -> AnyElement {
        self.choices(true, cx)
    }

    fn conversations(&mut self, cx: &mut Context<Self>) -> AnyElement {
        self.choices(false, cx)
    }

    fn review(&mut self, cx: &mut Context<Self>) -> AnyElement {
        self.sync_name(cx);
        let Some(wizard) = self.wizard.as_ref() else { return div().into_any_element() };
        let draft = &wizard.draft;
        let list = |items: &std::collections::BTreeSet<String>| -> SharedString {
            if items.is_empty() {
                tr("agent_wizard.none")
            } else {
                crate::i18n::list(&items.iter().collect::<Vec<_>>())
            }
        };
        let runtime: SharedString = if draft.runtime.is_empty() {
            tr("agent_wizard.bind_later")
        } else {
            draft.runtime.clone().into()
        };
        let facts = [
            (
                "agent-review-name",
                tr("agent_wizard.name"),
                SharedString::from(draft.name.trim().to_owned()),
            ),
            ("agent-review-runtime", tr("agent_wizard.step.runtime"), runtime),
            ("agent-review-tools", tr("agent_wizard.step.capabilities"), list(&draft.tools)),
            ("agent-review-scopes", tr("agent_wizard.step.conversations"), list(&draft.scopes)),
        ];
        v_flex()
            .id("agent-wizard-review")
            .test_support()
            .gap_2()
            .children(facts.into_iter().map(|(id, label, value)| {
                v_flex()
                    .id(id)
                    .test_support()
                    .gap_1()
                    .child(div().text_xs().text_color(cx.theme().muted_foreground).child(label))
                    .child(div().text_sm().whitespace_normal().child(value))
            }))
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .whitespace_normal()
                    .child(tr("agent_wizard.review_help")),
            )
            .into_any_element()
    }
}
