// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The AI participants section's parts: the introduction with its create
//! button (or the creation ceremony in its place), and the list of AI
//! participants with their export forms.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::StyledExt as _;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Context, Role, SharedString, Window, div};
use ramflux_desktop_core::{Account, AgentExportPhase, Networking};

use super::AgentsPanel;
use crate::i18n::{self, tr, tr_args};
use crate::ids::domain_id;

fn muted(text: SharedString, cx: &App) -> AnyElement {
    div().text_sm().text_color(cx.theme().muted_foreground).child(text).into_any_element()
}

/// A muted note a reader hears and the tests read.
fn note(id: &'static str, key: &str, cx: &App) -> AnyElement {
    let text = tr(key);
    div()
        .id(id)
        .role(Role::Status)
        .aria_label(text.clone())
        .test_support()
        .child(muted(text, cx))
        .into_any_element()
}

impl AgentsPanel {
    /// Why an AI participant cannot be created now (translated), if so.
    fn create_reason(&self, cx: &App) -> Option<SharedString> {
        if !self.store.read(cx).entry_store().offers_born_v3_creation() {
            return Some(tr("ceremony.ended.disabled"));
        }
        self.owner_problem(cx).map(|problem| i18n::agent_custody_problem(Some(problem)))
    }

    /// The introduction and the create button; the ceremony in their place
    /// while it is open.
    pub(super) fn render_create(&mut self, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if self.creating {
            return self.ceremony.clone().into_any_element();
        }
        let reason = self.create_reason(cx);
        v_flex()
            .gap_2()
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
                        .primary()
                        .label(tr("agent.settings.create"))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.begin_creating(window, cx);
                        })),
                    reason,
                )),
            )
            .into_any_element()
    }

    /// The AI participants, each with its kind, custodian, state and export.
    pub(super) fn render_list(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let agents = self.agents(cx);
        if agents.is_empty() {
            return div()
                .id("agent-empty")
                .role(Role::Status)
                .aria_label(tr("agent.settings.empty"))
                .test_support()
                .child(muted(tr("agent.settings.empty"), cx))
                .into_any_element();
        }
        v_flex()
            .id("agent-list")
            .role(Role::List)
            .aria_label(tr("agent.settings.list_title"))
            .test_support()
            .gap_3()
            .children(agents.iter().map(|agent| self.agent_row(agent, window, cx)))
            .into_any_element()
    }

    fn custodian_text(&self, agent: &Account, cx: &App) -> SharedString {
        let store = self.store.read(cx).entry_store();
        match store.custodian_of(&agent.id) {
            Some(custodian) => {
                let name = store
                    .account(custodian)
                    .map_or_else(|| custodian.to_string(), |account| account.label.clone());
                tr_args("agent.row.custodian", &[("name", &name)])
            }
            None => tr("agent.row.custodian_unknown"),
        }
    }

    fn agent_row(
        &mut self,
        agent: &Account,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = agent.id.clone();
        let open = self.exporting.as_ref() == Some(&id);
        let detail = tr_args(
            "agent.row.detail",
            &[
                ("kind", &i18n::account_kind(agent.kind)),
                ("custodian", &self.custodian_text(agent, cx)),
            ],
        );
        let state = match agent.networking {
            Networking::Live => Tag::success().small().child(tr("agent.row.live")),
            Networking::Pending => Tag::warning().small().child(tr("agent.row.pending")),
        };
        let exported = matches!(self.outcome(&id, cx), Some(AgentExportPhase::Exported));
        let reason =
            self.owner_problem(cx).map(|problem| i18n::agent_custody_problem(Some(problem)));
        let label = tr_args("agent.row.label", &[("name", &agent.label)]);
        let row = h_flex()
            .id(domain_id("agent", &id))
            .role(Role::ListItem)
            .aria_label(label)
            .test_support()
            .gap_3()
            .px_3()
            .py_2()
            .border_1()
            .border_color(cx.theme().border)
            .rounded(cx.theme().radius)
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_sm().font_semibold().child(agent.label.clone()))
                    .child(
                        div()
                            .id(domain_id("agent-detail", &id))
                            .role(Role::Status)
                            .aria_label(detail.clone())
                            .test_support()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(detail),
                    )
                    .child(
                        div()
                            .id(domain_id("agent-not-unlocked", &id))
                            .role(Role::Status)
                            .aria_label(tr("agent.row.not_unlocked"))
                            .test_support()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(tr("agent.row.not_unlocked")),
                    ),
            )
            .child(
                h_flex().gap_2().items_center().child(state).child(crate::gate::gated(
                    Button::new(domain_id("agent-export", &id))
                        .small()
                        .outline()
                        .label(tr("agent.export.button"))
                        .accessibility_label(tr_args(
                            "agent.export.title",
                            &[("name", &agent.label)],
                        ))
                        .on_click({
                            let id = id.clone();
                            cx.listener(move |this, _, window, cx| {
                                this.open_export(id.clone(), window, cx);
                            })
                        }),
                    reason,
                )),
            );
        v_flex()
            .gap_2()
            .child(row)
            .children(open.then(|| self.export_form(agent, exported, window, cx)))
            .into_any_element()
    }

    /// The export form of an AI participant and what came of it.
    fn export_form(
        &mut self,
        agent: &Account,
        exported: bool,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = agent.id.clone();
        let sending = self.is_exporting();
        let typed = !self.location.read(cx).value().trim().is_empty();
        let result = match self.outcome(&id, cx) {
            Some(AgentExportPhase::Exported) => {
                let path = self.exported_path(&id, cx).unwrap_or_default();
                let done = tr_args("agent.export.done", &[("path", &path)]);
                Some(
                    div()
                        .id("agent-export-result")
                        .role(Role::Status)
                        .aria_label(done.clone())
                        .test_support()
                        .child(
                            v_flex()
                                .gap_1()
                                .child(Alert::success("agent-export-done", done).small())
                                .child(note(
                                    "agent-export-done-next",
                                    "agent.export.done_next",
                                    cx,
                                )),
                        )
                        .into_any_element(),
                )
            }
            Some(AgentExportPhase::Refused(refusal)) => {
                let why = i18n::agent_export_refusal(refusal);
                Some(
                    div()
                        .id("agent-export-result")
                        .role(Role::Alert)
                        .aria_label(why.clone())
                        .test_support()
                        .child(Alert::error("agent-export-refused", why).small())
                        .into_any_element(),
                )
            }
            None => None,
        };
        v_flex()
            .id("agent-export-form")
            .test_support()
            .gap_2()
            .p_3()
            .border_1()
            .border_color(cx.theme().border)
            .rounded(cx.theme().radius)
            .child(
                div()
                    .text_sm()
                    .font_semibold()
                    .child(tr_args("agent.export.title", &[("name", &agent.label)])),
            )
            .child(note("agent-export-once", "agent.export.once", cx))
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(tr("agent.export.where")),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                div().flex_1().min_w_0().child(
                                    Input::new(&self.location)
                                        .id("agent-export-location")
                                        .aria_label(tr("agent.export.where"))
                                        .disabled(sending || exported),
                                ),
                            )
                            .child(
                                Button::new("agent-export-choose")
                                    .small()
                                    .outline()
                                    .label(tr("agent.export.choose"))
                                    .disabled(sending || exported)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.choose_location(window, cx);
                                    })),
                            ),
                    ),
            )
            .children(sending.then(|| {
                div()
                    .id("agent-export-working")
                    .role(Role::Status)
                    .aria_label(tr("agent.export.working"))
                    .test_support()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(Spinner::new().small())
                            .child(tr("agent.export.working")),
                    )
            }))
            .children(result)
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("agent-export-submit")
                            .small()
                            .primary()
                            .label(tr("agent.export.submit"))
                            .loading(sending)
                            .disabled(sending || exported || !typed)
                            .on_click(cx.listener(|this, _, window, cx| this.export(window, cx))),
                    )
                    .child(
                        Button::new("agent-export-close")
                            .small()
                            .outline()
                            .label(tr("agent.export.close"))
                            .on_click(cx.listener(|this, _, _, cx| this.close_export(cx))),
                    ),
            )
            .into_any_element()
    }
}
