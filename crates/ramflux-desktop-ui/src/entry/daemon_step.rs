// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The daemon step: no daemon answers yet.
//!
//! On a live source the step says where the desktop looks and offers
//! "Start local daemon" with the program and data folder it would use.
//! Nothing starts until the user presses it. Without a daemon program the
//! offer is disabled and an alert points to Settings › Diagnostics; a
//! failed or crashed start is shown with its reason and can be retried.
//! While a started daemon comes up the step shows its progress.
//!
//! Playing scenarios, the step only shows the wait.

use std::path::Path;

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, IconName, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Context, IntoElement, SharedString, div};
use ramflux_desktop_sdk::local::{DaemonBinary, LaunchError};

use super::EntryPage;
use crate::actions::{OpenDiagnosticsSettings, StartLocalDaemon};
use crate::i18n::{tr, tr_args};
use crate::launch::{LaunchFailure, LaunchState};

fn shown(path: &Path) -> String {
    path.display().to_string()
}

/// The localized reason a start failed.
pub fn failure_text(failure: &LaunchFailure) -> SharedString {
    match failure {
        LaunchFailure::Launch(LaunchError::BinaryMissing) => tr("entry.daemon.missing"),
        LaunchFailure::Launch(LaunchError::BinaryNotFound(path)) => {
            tr_args("entry.daemon.not_found", &[("path", &shown(path))])
        }
        LaunchFailure::Launch(LaunchError::NotExecutable(path)) => {
            tr_args("entry.daemon.not_executable", &[("path", &shown(path))])
        }
        LaunchFailure::Launch(LaunchError::NoDataDir) => tr("entry.daemon.no_data_dir"),
        LaunchFailure::Launch(LaunchError::Spawn(error)) => {
            tr_args("entry.daemon.spawn_failed", &[("error", error)])
        }
        LaunchFailure::Exited(code) => {
            let status =
                code.map_or_else(|| tr("entry.daemon.signal").to_string(), |c| c.to_string());
            tr_args("entry.daemon.exited", &[("status", &status)])
        }
    }
}

fn muted_line(id: &'static str, text: SharedString, mono: bool, cx: &App) -> AnyElement {
    div()
        .id(id)
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .when(mono, |line| line.font_family(cx.theme().mono_font_family.clone()))
        .child(text)
        .test_support()
        .into_any_element()
}

/// An alert under a stable, test-visible id.
fn alert_box(id: &'static str, alert: Alert) -> AnyElement {
    div().id(id).child(alert).test_support().into_any_element()
}

impl EntryPage {
    pub(super) fn render_daemon(&self, cx: &mut Context<Self>) -> AnyElement {
        let launcher = self.launcher.read(cx);
        let Some(socket) = launcher.socket(cx) else {
            return self.render_daemon_wait(cx);
        };
        let state = launcher.state().clone();
        let binary = launcher.binary();
        let data_dir = launcher.data_dir();
        let log = launcher.daemon_log();
        let header = v_flex()
            .gap_1()
            .child(
                div()
                    .text_sm()
                    .font_weight(gpui_kit::FontWeight::MEDIUM)
                    .child(tr("entry.daemon.none")),
            )
            .child(muted_line(
                "entry-daemon-socket",
                tr_args("entry.daemon.none_help", &[("path", &shown(&socket))]),
                false,
                cx,
            ));
        let body = match &state {
            LaunchState::Starting { .. }
            | LaunchState::Running { .. }
            | LaunchState::Stopping { .. } => {
                let key = if matches!(state, LaunchState::Starting { .. }) {
                    "entry.daemon.starting"
                } else {
                    "entry.daemon.connecting"
                };
                h_flex()
                    .id("entry-daemon-starting")
                    .gap_3()
                    .child(Spinner::new())
                    .child(v_flex().gap_1().child(div().text_sm().child(tr(key))).children(
                        log.map(|log| {
                            muted_line(
                                "entry-daemon-log",
                                tr_args("entry.daemon.output", &[("path", &shown(&log))]),
                                true,
                                cx,
                            )
                        }),
                    ))
                    .test_support()
                    .into_any_element()
            }
            LaunchState::Idle | LaunchState::Failed(_) => {
                self.render_daemon_offer(&state, &binary, data_dir.as_deref(), cx)
            }
        };
        v_flex()
            .id("entry-daemon")
            .gap_4()
            .child(header)
            .child(body)
            .test_support()
            .into_any_element()
    }

    fn render_daemon_offer(
        &self,
        state: &LaunchState,
        binary: &DaemonBinary,
        data_dir: Option<&Path>,
        cx: &App,
    ) -> AnyElement {
        let failure = match state {
            LaunchState::Failed(failure) => Some(failure),
            _ => None,
        };
        let missing = binary.path().is_none();
        let alert = if let Some(failure) = failure {
            Some(alert_box(
                "entry-daemon-failed",
                Alert::error("entry-daemon-failed-alert", failure_text(failure))
                    .title(tr("entry.daemon.failed_title")),
            ))
        } else if missing {
            Some(alert_box(
                "entry-daemon-missing",
                Alert::warning("entry-daemon-missing-alert", tr("entry.daemon.missing"))
                    .title(tr("entry.daemon.missing_title")),
            ))
        } else {
            None
        };
        let start_label =
            if failure.is_some() { "entry.daemon.retry" } else { "entry.daemon.start" };
        let paths = v_flex()
            .gap_1()
            .children(binary.path().map(|path| {
                muted_line(
                    "entry-daemon-binary",
                    tr_args("entry.daemon.runs", &[("path", &shown(path))]),
                    true,
                    cx,
                )
            }))
            .children(data_dir.map(|dir| {
                muted_line(
                    "entry-daemon-data",
                    tr_args("entry.daemon.data", &[("path", &shown(dir))]),
                    true,
                    cx,
                )
            }));
        v_flex()
            .gap_3()
            .children(alert)
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("entry-daemon-start")
                            .primary()
                            .icon(IconName::Play)
                            .label(tr(start_label))
                            .disabled(missing)
                            .on_click(|_, window, cx| {
                                window.dispatch_action(Box::new(StartLocalDaemon), cx);
                            }),
                    )
                    .child(
                        Button::new("entry-daemon-settings")
                            .ghost()
                            .icon(IconName::Settings)
                            .label(tr("entry.daemon.settings"))
                            .on_click(|_, window, cx| {
                                window.dispatch_action(Box::new(OpenDiagnosticsSettings), cx);
                            }),
                    ),
            )
            .child(paths)
            .child(muted_line(
                "entry-daemon-on-request",
                tr("entry.daemon.only_on_request"),
                false,
                cx,
            ))
            .into_any_element()
    }

    /// Scenarios: the daemon is simulated; only the wait is shown.
    fn render_daemon_wait(&self, cx: &App) -> AnyElement {
        h_flex()
            .id("entry-daemon")
            .gap_3()
            .child(Spinner::new())
            .child(
                v_flex().gap_1().child(div().text_sm().child(tr("entry.daemon.waiting"))).child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(tr("entry.daemon.help")),
                ),
            )
            .test_support()
            .into_any_element()
    }
}
