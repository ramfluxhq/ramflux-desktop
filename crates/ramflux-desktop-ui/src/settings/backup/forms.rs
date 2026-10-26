// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The backup form (where, and a passphrase typed twice with a strength
//! hint) and the restore form (which file, and its passphrase). Each opens
//! from its group's button; a request the daemon refused is shown in the
//! form, next to the button that sent it.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Context, Entity, Role, SharedString, Window, div};
use ramflux_desktop_core::{BackupJobKind, BackupPhase, methods};

use super::text::{self, Strength, count};
use super::{BackupPanel, Form};
use crate::file_dialog::{self, StartDir};
use crate::i18n::{self, tr, tr_args};

/// A labelled field.
fn field(label: SharedString, body: impl IntoElement, cx: &App) -> AnyElement {
    v_flex()
        .gap_1()
        .child(div().text_xs().text_color(cx.theme().muted_foreground).child(label))
        .child(body)
        .into_any_element()
}

fn masked(input: &Entity<InputState>, id: &'static str, label: SharedString) -> Input {
    Input::new(input).id(id).aria_label(label).mask_toggle()
}

/// A chosen path, or a muted note that none is chosen yet.
fn chosen(path: Option<&String>, none: &str, cx: &App) -> AnyElement {
    let text = div().flex_1().min_w_0().truncate();
    match path {
        Some(path) => text.child(SharedString::from(path.clone())).into_any_element(),
        None => text.text_color(cx.theme().muted_foreground).child(tr(none)).into_any_element(),
    }
}

fn muted(key: &str, cx: &App) -> AnyElement {
    div().text_sm().text_color(cx.theme().muted_foreground).child(tr(key)).into_any_element()
}

impl BackupPanel {
    fn reason(&self, method: &str, cx: &App) -> Option<SharedString> {
        crate::gate::reason(self.store.read(cx).store(), method)
    }

    fn refusal(&self, cx: &App) -> Option<AnyElement> {
        self.refused(cx).map(|failure| {
            let why = text::refusal(&failure.message);
            div()
                .id("backup-refused")
                .role(Role::Alert)
                .aria_label(why.clone())
                .test_support()
                .child(Alert::error("backup-refused-alert", why).small())
                .into_any_element()
        })
    }

    pub(super) fn render_backup(&mut self, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let reason = self.reason(methods::BACKUP_CREATE, cx);
        if self.form != Some(Form::Backup) {
            let last = self
                .store
                .read(cx)
                .store()
                .backup_jobs()
                .filter(|j| j.kind == BackupJobKind::Backup && j.phase == BackupPhase::Done)
                .max_by_key(|j| (j.started_at, self.store.read(cx).store().backup_job_order(&j.id)))
                .map(|j| {
                    let size = j.summary.as_ref().map_or(0, |s| s.file_bytes);
                    tr_args(
                        "settings.backup.last",
                        &[
                            ("time", &i18n::date_time(j.finished_at.unwrap_or(j.started_at))),
                            ("size", &i18n::bytes(size)),
                        ],
                    )
                });
            return v_flex()
                .gap_2()
                .child(muted("settings.backup.create_help", cx))
                .children(last.map(|last| {
                    div().text_sm().text_color(cx.theme().muted_foreground).child(last)
                }))
                .child(
                    h_flex().child(crate::gate::gated(
                        Button::new("backup-open")
                            .small()
                            .label(tr("settings.backup.create"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open(Form::Backup, window, cx);
                            })),
                        reason,
                    )),
                )
                .into_any_element();
        }
        let typed = self.passphrase.read(cx).value();
        let repeated = self.repeat.read(cx).value();
        let strength = Strength::of(&typed);
        let mismatch = !repeated.is_empty() && repeated != typed;
        let hint_color = match strength {
            Strength::TooShort | Strength::Weak => cx.theme().warning,
            Strength::Fair | Strength::Strong => cx.theme().muted_foreground,
        };
        let suggested =
            format!("ramflux-backup-{}.rfbackup", chrono::Local::now().format("%Y-%m-%d"));
        let ready = self.backup_ready(cx) && reason.is_none();
        v_flex()
            .gap_3()
            .child(field(
                tr("settings.backup.location"),
                h_flex()
                    .gap_2()
                    .child(chosen(self.location.as_ref(), "settings.backup.no_location", cx))
                    .child(
                        Button::new("backup-choose-location")
                            .small()
                            .outline()
                            .label(tr("settings.backup.choose"))
                            .on_click(cx.listener(move |_, _, window, cx| {
                                let panel = cx.entity();
                                file_dialog::save_as(
                                    StartDir::Documents,
                                    &suggested,
                                    window,
                                    cx,
                                    move |path, _, cx| {
                                        panel.update(cx, |panel, cx| panel.set_location(path, cx));
                                    },
                                );
                            })),
                    ),
                cx,
            ))
            .children(self.location_exists.then(|| {
                div()
                    .id("backup-location-exists")
                    .role(Role::Alert)
                    .aria_label(tr("settings.backup.location_exists"))
                    .test_support()
                    .text_sm()
                    .text_color(cx.theme().danger)
                    .child(tr("settings.backup.location_exists"))
            }))
            .child(field(
                tr("settings.backup.passphrase"),
                masked(&self.passphrase, "backup-new-passphrase", tr("settings.backup.passphrase")),
                cx,
            ))
            .child(field(
                tr("settings.backup.repeat"),
                masked(&self.repeat, "backup-repeat-passphrase", tr("settings.backup.repeat")),
                cx,
            ))
            .child(
                v_flex()
                    .gap_1()
                    .text_sm()
                    .children((!typed.is_empty()).then(|| {
                        div()
                            .id("backup-strength")
                            .role(Role::Status)
                            .aria_label(strength.hint())
                            .test_support()
                            .text_color(hint_color)
                            .child(strength.hint())
                    }))
                    .children(mismatch.then(|| {
                        div()
                            .id("backup-mismatch")
                            .role(Role::Alert)
                            .aria_label(tr("settings.backup.mismatch"))
                            .test_support()
                            .text_color(cx.theme().danger)
                            .child(tr("settings.backup.mismatch"))
                    }))
                    .child(muted("settings.backup.passphrase_help", cx)),
            )
            .children(self.allow_missing.map(|files| {
                let note = tr_args("settings.backup.missing.note", &[("count", &count(files))]);
                div()
                    .id("backup-allow-missing-note")
                    .test_support()
                    .child(Alert::warning("backup-allow-missing-alert", note).small())
            }))
            .children(self.refusal(cx))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("backup-form-cancel")
                            .small()
                            .outline()
                            .label(tr("common.cancel"))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.close_form(window, cx)),
                            ),
                    )
                    .child(crate::gate::gated(
                        Button::new("backup-start")
                            .small()
                            .primary()
                            .label(tr("settings.backup.start"))
                            .disabled(!ready)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.start_backup(window, cx);
                            })),
                        reason,
                    )),
            )
            .into_any_element()
    }

    pub(super) fn render_restore(&mut self, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let reason = self.reason(methods::RESTORE_VERIFY, cx);
        if self.form != Some(Form::Restore) {
            return v_flex()
                .gap_2()
                .child(muted("settings.backup.restore_help", cx))
                .child(
                    h_flex().child(crate::gate::gated(
                        Button::new("restore-open")
                            .small()
                            .label(tr("settings.backup.restore"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open(Form::Restore, window, cx);
                            })),
                        reason,
                    )),
                )
                .into_any_element();
        }
        let ready = self.file.is_some()
            && !self.restore_secret.read(cx).value().is_empty()
            && reason.is_none();
        v_flex()
            .gap_3()
            .child(field(
                tr("settings.backup.file"),
                h_flex()
                    .gap_2()
                    .child(chosen(self.file.as_ref(), "settings.backup.no_file", cx))
                    .child(
                        Button::new("restore-choose-file")
                            .small()
                            .outline()
                            .label(tr("settings.backup.choose"))
                            .on_click(cx.listener(|_, _, window, cx| {
                                let panel = cx.entity();
                                file_dialog::open_file(
                                    tr("settings.backup.file"),
                                    window,
                                    cx,
                                    move |path, _, cx| {
                                        panel.update(cx, |panel, cx| panel.set_file(path, cx));
                                    },
                                );
                            })),
                    ),
                cx,
            ))
            .child(field(
                tr("settings.backup.passphrase"),
                masked(
                    &self.restore_secret,
                    "restore-passphrase",
                    tr("settings.backup.passphrase"),
                ),
                cx,
            ))
            .child(muted("settings.backup.verify_help", cx))
            .children(self.refusal(cx))
            .child(
                h_flex()
                    .gap_2()
                    .children((!self.standalone).then(|| {
                        Button::new("restore-form-cancel")
                            .small()
                            .outline()
                            .label(tr("common.cancel"))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.close_form(window, cx)),
                            )
                    }))
                    .child(crate::gate::gated(
                        Button::new("restore-verify")
                            .small()
                            .primary()
                            .label(tr("settings.backup.verify"))
                            .disabled(!ready)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.start_restore(window, cx);
                            })),
                        reason,
                    )),
            )
            .into_any_element()
    }
}
