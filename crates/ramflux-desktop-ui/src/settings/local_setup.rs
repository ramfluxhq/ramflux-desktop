// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Settings · Diagnostics, local setup: where the desktop gets its data
//! (source, daemon socket), the daemon program and data folder it may start
//! a daemon with, the local daemon itself, and version and log locations.
//!
//! These are local preferences kept in the desktop settings file, never
//! account data. Paths are typed or chosen with the native dialog and saved
//! together; an empty field means the default shown as its placeholder. The
//! source switch saves at once. Source and socket apply the next time the
//! application starts; program and data folder apply to the next start of
//! a local daemon.

use std::path::{Path, PathBuf};

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::description_list::DescriptionList;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::setting::{SettingGroup, SettingItem};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Entity, SharedString, Window, div};
use ramflux_desktop_core::SourceStatus;
use ramflux_desktop_sdk::local::{LaunchSettings, SourceKind, app_log};

use super::SectionContext;
use crate::actions::{StartLocalDaemon, StopLocalDaemon};
use crate::entry::daemon_failure_text;
use crate::file_dialog;
use crate::i18n::{tr, tr_args};
use crate::launch::{DaemonLauncher, LaunchState, version};

/// The path inputs of the local setup, owned by the settings page.
pub struct LaunchInputs {
    /// Daemon socket.
    pub socket: Entity<InputState>,
    /// Daemon program.
    pub binary: Entity<InputState>,
    /// Data folder of a started daemon.
    pub data_dir: Entity<InputState>,
}

fn shown(path: &Path) -> String {
    path.display().to_string()
}

impl LaunchInputs {
    /// Inputs filled with the saved choices; placeholders show the defaults.
    pub fn new(launcher: &Entity<DaemonLauncher>, window: &mut Window, cx: &mut App) -> Self {
        let (saved, bundled) = {
            let launcher = launcher.read(cx);
            (launcher.settings().clone(), launcher.bundled().map(shown))
        };
        let socket_default = shown(&LaunchSettings::default().socket_or_default());
        let data_default = LaunchSettings::default()
            .data_dir_or_default()
            .map_or_else(String::new, |dir| shown(&dir));
        let input = |value: Option<&PathBuf>,
                     placeholder: SharedString,
                     window: &mut Window,
                     cx: &mut App| {
            let value = value.map(|p| shown(p)).unwrap_or_default();
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder).default_value(value))
        };
        // Without a bundled daemon the placeholder is a translated hint.
        let binary = match bundled {
            Some(path) => input(saved.daemon_binary.as_ref(), path.into(), window, cx),
            None => {
                let value = saved.daemon_binary.as_ref().map(|p| shown(p)).unwrap_or_default();
                crate::i18n::localized_input(
                    "settings.launch.binary_placeholder",
                    window,
                    cx,
                    |input| input.default_value(value),
                )
            }
        };
        Self {
            socket: input(saved.socket_path.as_ref(), socket_default.into(), window, cx),
            binary,
            data_dir: input(saved.data_dir.as_ref(), data_default.into(), window, cx),
        }
    }

    /// `current` with the typed paths (an empty field is the default).
    fn read(&self, current: &LaunchSettings, cx: &App) -> LaunchSettings {
        let path = |input: &Entity<InputState>| {
            let text = input.read(cx).value().trim().to_owned();
            (!text.is_empty()).then(|| PathBuf::from(text))
        };
        LaunchSettings {
            socket_path: path(&self.socket),
            daemon_binary: path(&self.binary),
            data_dir: path(&self.data_dir),
            ..current.clone()
        }
    }
}

fn label(key: &str) -> impl IntoElement {
    div().text_sm().child(tr(key))
}

fn muted(id: &'static str, text: SharedString, cx: &App) -> AnyElement {
    div()
        .id(id)
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(text)
        .test_support()
        .into_any_element()
}

fn path_field(
    id: &'static str,
    key: &'static str,
    input: &Entity<InputState>,
    browse: Option<(&'static str, bool)>,
) -> AnyElement {
    let browse = browse.map(|(button_id, folder)| {
        let input = input.clone();
        Button::new(button_id).small().outline().label(tr("settings.launch.browse")).on_click(
            move |_, window, cx| {
                let input = input.clone();
                let fill = move |path: String, window: &mut Window, cx: &mut App| {
                    input.update(cx, |input, cx| input.set_value(path, window, cx));
                };
                let prompt = tr(key);
                if folder {
                    file_dialog::open_dir(prompt, window, cx, fill);
                } else {
                    file_dialog::open_file(prompt, window, cx, fill);
                }
            },
        )
    });
    v_flex()
        .gap_1()
        .child(label(key))
        .child(
            h_flex()
                .gap_2()
                .child(div().flex_1().min_w_0().child(Input::new(input).id(id).aria_label(tr(key))))
                .children(browse),
        )
        .into_any_element()
}

fn setup_group(context: &SectionContext, inputs: &LaunchInputs, cx: &App) -> SettingGroup {
    let launcher = context.launcher.read(cx);
    // The saved choice, else what this window runs on.
    let use_daemon = launcher
        .settings()
        .source
        .map_or_else(|| context.store.read(cx).is_live(), |kind| kind == SourceKind::Rfd);
    let saved = launcher.last_save().cloned();
    let source = {
        let launcher = context.launcher.clone();
        SettingItem::render(move |_, _, cx: &mut App| {
            let launcher = launcher.clone();
            v_flex()
                .gap_1()
                .child(
                    Switch::new("launch-source")
                        .label(tr("settings.launch.use_daemon"))
                        .checked(use_daemon)
                        .on_change(move |checked, _, cx| {
                            let kind = if *checked { SourceKind::Rfd } else { SourceKind::Mock };
                            launcher.update(cx, |launcher, cx| launcher.set_source(kind, cx));
                        }),
                )
                .child(muted("launch-source-help", tr("settings.launch.source_help"), cx))
                .into_any_element()
        })
    };
    let paths = {
        let (socket, binary, data_dir) =
            (inputs.socket.clone(), inputs.binary.clone(), inputs.data_dir.clone());
        let launcher = context.launcher.clone();
        SettingItem::render(move |_, _, cx: &mut App| {
            let inputs = LaunchInputs {
                socket: socket.clone(),
                binary: binary.clone(),
                data_dir: data_dir.clone(),
            };
            let status = match &saved {
                Some(Ok(())) => Some(muted("launch-saved", tr("settings.launch.saved"), cx)),
                Some(Err(error)) => Some(
                    div()
                        .id("launch-save-error")
                        .text_xs()
                        .text_color(cx.theme().danger)
                        .child(tr_args("settings.launch.save_failed", &[("error", error)]))
                        .test_support()
                        .into_any_element(),
                ),
                None => None,
            };
            let launcher = launcher.clone();
            v_flex()
                .gap_3()
                .child(path_field("launch-socket", "settings.launch.socket", &inputs.socket, None))
                .child(path_field(
                    "launch-binary",
                    "settings.launch.binary",
                    &inputs.binary,
                    Some(("launch-binary-browse", false)),
                ))
                .child(path_field(
                    "launch-data-dir",
                    "settings.launch.data_dir",
                    &inputs.data_dir,
                    Some(("launch-data-dir-browse", true)),
                ))
                .child(
                    h_flex()
                        .gap_3()
                        .items_center()
                        .child(
                            Button::new("launch-save")
                                .small()
                                .primary()
                                .label(tr("settings.launch.save"))
                                .on_click(move |_, _, cx| {
                                    let current = launcher.read(cx).settings().clone();
                                    let next = inputs.read(&current, cx);
                                    launcher.update(cx, |launcher, cx| launcher.save(next, cx));
                                }),
                        )
                        .children(status),
                )
                .child(muted("launch-apply-help", tr("settings.launch.apply_help"), cx))
                .into_any_element()
        })
    };
    SettingGroup::new().title(tr("settings.launch.title")).item(source).item(paths)
}

fn daemon_status(context: &SectionContext, cx: &App) -> SharedString {
    let launcher = context.launcher.read(cx);
    let handle = context.store.read(cx);
    let socket = handle.live_socket().unwrap_or_default().to_owned();
    match launcher.state() {
        LaunchState::Starting { pid } | LaunchState::Running { pid }
            if handle.store().source_status() != SourceStatus::Disconnected =>
        {
            tr_args("settings.launch.status_owned", &[("pid", &pid.to_string())])
        }
        LaunchState::Starting { .. } | LaunchState::Running { .. } => tr("entry.daemon.starting"),
        LaunchState::Stopping { .. } => tr("settings.launch.status_stopping"),
        LaunchState::Failed(failure) => daemon_failure_text(failure),
        LaunchState::Idle if !handle.is_live() => tr("settings.launch.status_scenarios"),
        LaunchState::Idle if launcher.left_behind(cx).is_some() => {
            let pid = launcher.left_behind(cx).unwrap_or_default();
            tr_args("settings.launch.status_left_behind", &[("pid", &pid.to_string())])
        }
        LaunchState::Idle if handle.store().source_status() != SourceStatus::Disconnected => {
            tr_args("settings.launch.status_attached", &[("path", &socket)])
        }
        LaunchState::Idle => tr_args("settings.launch.status_none", &[("path", &socket)]),
    }
}

fn daemon_group(context: &SectionContext, cx: &App) -> SettingGroup {
    let status = daemon_status(context, cx);
    let launcher = context.launcher.read(cx);
    let can_start = launcher.can_start(cx) && launcher.binary().path().is_some();
    let can_stop = launcher.can_stop(cx);
    let item = SettingItem::render(move |_, _, cx: &mut App| {
        v_flex()
            .gap_2()
            .child(div().id("launch-daemon-status").text_sm().child(status.clone()).test_support())
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("launch-daemon-start")
                            .small()
                            .outline()
                            .label(tr("entry.daemon.start"))
                            .disabled(!can_start)
                            .on_click(|_, window, cx| {
                                window.dispatch_action(Box::new(StartLocalDaemon), cx);
                            }),
                    )
                    .when(can_stop, |row| {
                        row.child(
                            Button::new("launch-daemon-stop")
                                .small()
                                .outline()
                                .label(tr("settings.launch.stop"))
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(StopLocalDaemon), cx);
                                }),
                        )
                    }),
            )
            .child(muted("launch-daemon-help", tr("entry.daemon.only_on_request"), cx))
            .into_any_element()
    });
    SettingGroup::new().title(tr("settings.launch.daemon_title")).item(item)
}

fn about_group(context: &SectionContext, cx: &App) -> SettingGroup {
    let launcher = context.launcher.read(cx);
    let none = tr("settings.launch.none");
    let path_text = |path: Option<PathBuf>| {
        path.map_or_else(|| none.clone(), |p| SharedString::from(shown(&p)))
    };
    let rows = [
        ("settings.launch.version", SharedString::from(version::summary())),
        ("settings.launch.app_log", path_text(app_log::path())),
        ("settings.launch.daemon_log", path_text(launcher.daemon_log())),
        ("settings.launch.settings_file", path_text(launcher.settings_file())),
    ];
    let item = SettingItem::render(move |_, _, _: &mut App| {
        rows.iter()
            .fold(DescriptionList::new().columns(1), |list, (key, value)| {
                list.item(tr(key), value.clone(), 1)
            })
            .into_any_element()
    });
    SettingGroup::new().title(tr("settings.launch.about_title")).item(item)
}

/// The local setup groups of the diagnostics section, in order: startup
/// choices, the local daemon, version and logs.
pub fn groups(context: &SectionContext, inputs: &LaunchInputs, cx: &App) -> [SettingGroup; 3] {
    [setup_group(context, inputs, cx), daemon_group(context, cx), about_group(context, cx)]
}
