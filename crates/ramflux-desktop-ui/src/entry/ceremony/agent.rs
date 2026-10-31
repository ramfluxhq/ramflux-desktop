// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The screens only an AI participant's ceremony has (DESK-AGENT-01): the
//! name step with its custodian, the wait while the daemon creates the
//! account, and the result. The screens in between (get ready, the words,
//! the three words asked) are the account ceremony's own.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Context, Role, Window, div};
use ramflux_desktop_core::{AccountId, CeremonyAccount, Networking};

use super::{CeremonyPanel, Pending};
use crate::entry::parts::{announced, muted};
use crate::i18n::{self, tr, tr_args};

impl CeremonyPanel {
    /// The name the owner's account is listed under (its id when unknown).
    fn custodian_name(&self, id: &AccountId, cx: &gpui_kit::App) -> String {
        self.store
            .read(cx)
            .entry_store()
            .account(id)
            .map_or_else(|| id.to_string(), |account| account.label.clone())
    }

    /// The name step of an AI participant: who custodies it, what the
    /// ceremony does differently from an account's, and why it cannot begin
    /// when it cannot.
    pub(super) fn agent_name_screen(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let typed = !self.name.read(cx).value().trim().is_empty();
        let problem = self.custody_problem(cx);
        let custodian = match (&self.custodian, problem) {
            (Some(id), None) => announced(
                "agent-custodian",
                Role::Status,
                tr_args("agent.create.custodian", &[("name", &self.custodian_name(id, cx))]),
                muted(
                    tr_args("agent.create.custodian", &[("name", &self.custodian_name(id, cx))]),
                    cx,
                ),
            ),
            (_, problem) => {
                let text = i18n::agent_custody_problem(problem);
                announced(
                    "agent-custodian-problem",
                    Role::Alert,
                    text.clone(),
                    Alert::error("agent-custodian-problem-alert", text).small(),
                )
            }
        };
        v_flex()
            .id("ceremony-name")
            .test_support()
            .gap_3()
            .child(self.heading("agent.create.title", window, cx))
            .child(muted(tr("agent.create.help"), cx))
            .child(custodian)
            .child(
                v_flex().gap_1().child(muted(tr("agent.create.label"), cx)).child(
                    Input::new(&self.name)
                        .id("ceremony-name-input")
                        .aria_label(tr("agent.create.label")),
                ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        self.sending(
                            Button::new("ceremony-continue")
                                .primary()
                                .label(tr("ceremony.continue"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.begin(window, cx);
                                })),
                            Pending::Begin,
                        )
                        .loading(self.pending == Some(Pending::Begin) || self.is_abandoned())
                        .disabled(
                            self.pending.is_some()
                                || self.is_abandoned()
                                || !typed
                                || self.custody_problem(cx).is_some(),
                        ),
                    )
                    .child(self.cancel_button("ceremony.back", cx)),
            )
            .into_any_element()
    }

    /// The three words were sent: the daemon is creating the account.
    pub(super) fn creating_screen(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .id("ceremony-creating")
            .test_support()
            .gap_3()
            .child(self.heading("agent.creating.title", window, cx))
            .child(
                div()
                    .id("ceremony-creating-text")
                    .role(Role::Status)
                    .aria_label(tr("agent.creating.text"))
                    .test_support()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(Spinner::new().small())
                            .child(tr("agent.creating.text")),
                    ),
            )
            .into_any_element()
    }

    /// The AI participant exists. It is not unlocked here: the agent host
    /// starts it from the credentials the owner exports.
    pub(super) fn agent_done_screen(
        &self,
        account: &CeremonyAccount,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let networking = self
            .store
            .read(cx)
            .entry_store()
            .account(&AccountId::new(&account.local_account_id))
            .map_or(account.networking, |listed| listed.networking);
        let live = networking == Networking::Live;
        let (state, state_text) = if live {
            ("ceremony.done.live_state", "ceremony.done.live_state_text")
        } else {
            ("ceremony.done.state", "ceremony.done.state_text")
        };
        let text = tr_args("agent.done.text", &[("name", &account.display_name)]);
        v_flex()
            .id("ceremony-done")
            .test_support()
            .gap_3()
            .child(self.heading("agent.done.title", window, cx))
            .child(announced(
                "ceremony-state",
                Role::Status,
                tr(state_text),
                h_flex().child(if live {
                    Tag::success().child(tr(state))
                } else {
                    Tag::warning().child(tr(state))
                }),
            ))
            .child(announced(
                "ceremony-done-text",
                Role::Status,
                text.clone(),
                Alert::success("ceremony-done-alert", text).small(),
            ))
            .child(announced(
                "agent-done-next",
                Role::Status,
                tr("agent.done.next"),
                muted(tr("agent.done.next"), cx),
            ))
            .child(
                h_flex().child(
                    Button::new("ceremony-done-close")
                        .primary()
                        .label(tr("ceremony.done.close"))
                        .on_click(cx.listener(|this, _, window, cx| this.close(window, cx))),
                ),
            )
            .into_any_element()
    }
}
