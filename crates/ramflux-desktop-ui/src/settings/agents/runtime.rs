// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The PRX runtime panel (derived #32): its state on this computer, and the
//! install, status, restart, stop and uninstall actions. The provider key
//! is typed into a masked field (shown as dots, kept out of the clipboard;
//! the eye toggle only reveals it on request), taken once into a [`SecretInput`] for the
//! install, and cleared from the field at once. It is never shown again,
//! never logged and never kept by this panel.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::v_flex;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Context, Entity, Role, SharedString, Window, div};
use ramflux_desktop_core::{
    AgentHostState, AgentRuntimeAction, AgentUnavailable, IntentKind, SecretInput, Store,
};
use ramflux_desktop_navigation::direction::h_flex;
use ramflux_desktop_navigation::scale::{space, text_caption, text_secondary};
use ramflux_desktop_sdk::translate::agents::{RESTART_HINT_KEYS, refusal_key};

use super::AgentsPanel;
use super::rows::unavailable_line;
use crate::i18n::{tr, tr_args};
use crate::root::{CONFIRMATION_REQUIRED, SUBMITTED};
use crate::style::DesignButton as _;

/// The install form's fields.
pub struct Inputs {
    pub provider: Entity<InputState>,
    pub model: Entity<InputState>,
    pub key: Entity<InputState>,
}

impl Inputs {
    /// The fields, empty.
    pub fn new(window: &mut Window, cx: &mut Context<AgentsPanel>) -> Self {
        Self {
            provider: crate::i18n::localized_input(
                "agent_wizard.provider_placeholder",
                window,
                cx,
                |i| i,
            ),
            model: crate::i18n::localized_input(
                "agent_wizard.model_placeholder",
                window,
                cx,
                |i| i,
            ),
            // Masked: the text is dots, and a masked field stays out of the clipboard.
            key: crate::i18n::localized_input("agents.runtime.key_placeholder", window, cx, |i| {
                i.masked(true)
            }),
        }
    }
}

/// One line of the runtime's state, for the panel and the wizard.
pub fn status_line(store: &Store) -> SharedString {
    let Some(host) = store.agent_host() else { return tr("agents.runtime.checking") };
    match host.state {
        AgentHostState::Unknown => tr("agents.runtime.checking"),
        AgentHostState::Absent => tr("agents.runtime.absent"),
        AgentHostState::Installing => tr("agents.runtime.installing"),
        AgentHostState::Stopped => tr("agents.runtime.stopped"),
        AgentHostState::Stale => tr("agents.runtime.stale"),
        AgentHostState::Running => match host.prx_version.as_deref() {
            Some(version) => tr_args("agents.runtime.running", &[("version", version)]),
            None => tr("agents.runtime.running_unknown_version"),
        },
    }
}

impl AgentsPanel {
    /// The runtime panel: its state, the install form when it is absent,
    /// and the actions the state allows.
    pub(super) fn render_runtime(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let state =
            self.store.read(cx).store().agent_host().map(|host| (host.state, host.refusal.clone()));
        // An install the panel submitted shows as installing until its answer.
        let installing = self.installing();
        let line = if installing {
            tr("agents.runtime.installing")
        } else {
            status_line(self.store.read(cx).store())
        };
        let refusal = state.as_ref().and_then(|(_, refusal)| refusal.clone());
        let host_state = if installing {
            Some(AgentHostState::Installing)
        } else {
            state.map(|(state, _)| state)
        };
        let restart_hint =
            self.last_failure(cx).is_some_and(|key| RESTART_HINT_KEYS.contains(&key.as_str()));
        let can_install = !self.runtime_inputs.provider.read(cx).value().trim().is_empty()
            && !self.runtime_inputs.model.read(cx).value().trim().is_empty()
            && !self.runtime_inputs.key.read(cx).value().trim().is_empty();
        v_flex()
            .id("agent-runtime")
            .test_support()
            .role(Role::Group)
            .aria_label(tr("agents.runtime.title"))
            .gap(space(2.))
            .child(
                h_flex()
                    .gap(space(2.))
                    .items_center()
                    .text_size(text_secondary())
                    .id("agent-runtime-status")
                    .role(Role::Status)
                    .aria_label(line.clone())
                    .test_support()
                    .children(
                        (host_state == Some(AgentHostState::Installing))
                            .then(|| Spinner::new().small()),
                    )
                    .child(line),
            )
            .children(refusal.map(|refusal| {
                let text = tr(refusal_key(&refusal));
                div()
                    .id("agent-runtime-refusal")
                    .role(Role::Alert)
                    .aria_label(text.clone())
                    .test_support()
                    .child(Alert::error("agent-runtime-refusal-alert", text).small())
            }))
            .children(restart_hint.then(|| {
                v_flex()
                    .gap(space(1.))
                    .child(
                        div()
                            .id("agent-runtime-restart-hint")
                            .text_size(text_caption())
                            .text_color(cx.theme().muted_foreground)
                            .whitespace_normal()
                            .child(tr("agents.runtime.restart_hint")),
                    )
                    .child(self.runtime_button(
                        "agent-runtime-restart-now",
                        "agents.runtime.restart",
                        AgentRuntimeAction::Restart,
                        true,
                        cx,
                    ))
            }))
            .children((host_state == Some(AgentHostState::Absent)).then(|| {
                v_flex()
                    .gap(space(2.))
                    .child(
                        div()
                            .text_size(text_caption())
                            .text_color(cx.theme().muted_foreground)
                            .whitespace_normal()
                            .child(tr("agents.runtime.install_help")),
                    )
                    .child(
                        Input::new(&self.runtime_inputs.provider)
                            .id("agent-runtime-provider")
                            .aria_label(tr("agent_wizard.provider")),
                    )
                    .child(
                        Input::new(&self.runtime_inputs.model)
                            .id("agent-runtime-model")
                            .aria_label(tr("agent_wizard.model")),
                    )
                    .child(
                        Input::new(&self.runtime_inputs.key)
                            .id("agent-runtime-key")
                            .aria_label(tr("agents.runtime.key"))
                            .mask_toggle(),
                    )
                    .child(
                        Button::new("agent-runtime-install")
                            .small()
                            .design_primary(cx)
                            .design_label(tr("agents.runtime.install"))
                            .design_disabled(!can_install, cx)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.install_runtime(window, cx)),
                            ),
                    )
            }))
            .child(self.runtime_actions(host_state, cx))
            .child(unavailable_line("agent-runtime-update", AgentUnavailable::RuntimeUpdate, cx))
            .into_any_element()
    }

    /// The actions the runtime state allows: status, restart, stop and
    /// uninstall. Restart is always there once a runtime is installed.
    fn runtime_actions(
        &mut self,
        state: Option<AgentHostState>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let installed = matches!(
            state,
            Some(AgentHostState::Running | AgentHostState::Stopped | AgentHostState::Stale)
        );
        let running = state == Some(AgentHostState::Running);
        let busy = state == Some(AgentHostState::Installing);
        h_flex()
            .gap(space(2.))
            .flex_wrap()
            .child(self.runtime_button(
                "agent-runtime-status-check",
                "agents.runtime.check",
                AgentRuntimeAction::Status,
                !busy,
                cx,
            ))
            .children(installed.then(|| {
                self.runtime_button(
                    "agent-runtime-restart",
                    "agents.runtime.restart",
                    AgentRuntimeAction::Restart,
                    !busy,
                    cx,
                )
            }))
            .children(running.then(|| {
                self.runtime_button(
                    "agent-runtime-stop",
                    "agents.runtime.stop",
                    AgentRuntimeAction::Stop,
                    !busy,
                    cx,
                )
            }))
            .children(installed.then(|| {
                self.runtime_button(
                    "agent-runtime-uninstall",
                    "agents.runtime.uninstall",
                    AgentRuntimeAction::Uninstall,
                    !busy,
                    cx,
                )
            }))
            .into_any_element()
    }

    fn runtime_button(
        &mut self,
        id: &'static str,
        key: &'static str,
        action: AgentRuntimeAction,
        enabled: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        Button::new(id)
            .small()
            .design_secondary(cx)
            .design_label(tr(key))
            .design_disabled(!enabled, cx)
            .on_click(
                cx.listener(move |this, _, window, cx| this.runtime_action(action, window, cx)),
            )
            .into_any_element()
    }

    /// Install: the key is taken from its field once and cleared there.
    fn install_runtime(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let provider = self.runtime_inputs.provider.read(cx).value().trim().to_owned();
        let model = self.runtime_inputs.model.read(cx).value().trim().to_owned();
        let key = SecretInput::new(self.runtime_inputs.key.read(cx).value().trim().to_owned());
        self.runtime_inputs.key.update(cx, |input, cx| input.set_value("", window, cx));
        let nonce = self.next_host_nonce();
        let kind = IntentKind::AgentRuntime {
            action: AgentRuntimeAction::Install,
            provider,
            model,
            key: Some(key),
            nonce,
        };
        self.arm_host_action(nonce, true);
        self.submit_host(kind, window, cx);
    }

    /// A runtime action without a key (status, restart, stop, uninstall).
    fn runtime_action(
        &mut self,
        action: AgentRuntimeAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let nonce = self.next_host_nonce();
        let kind = IntentKind::AgentRuntime {
            action,
            provider: String::new(),
            model: String::new(),
            key: None,
            nonce,
        };
        self.arm_host_action(nonce, false);
        self.submit_host(kind, window, cx);
    }

    /// Submits a runtime action through the root. One held for the owner's
    /// confirmation waits for it; one that was not submitted (refused, no
    /// account) is not followed: no answer will come for it.
    fn submit_host(&mut self, kind: IntentKind, window: &mut Window, cx: &mut Context<Self>) {
        let result = self.root.update(cx, |root, cx| root.submit_secret(kind, window, cx));
        if result == CONFIRMATION_REQUIRED {
            if let Some(action) = self.host_action.as_mut() {
                action.await_confirmation();
            }
        } else if !result.starts_with(SUBMITTED) {
            self.host_action = None;
        }
        cx.notify();
    }
}
