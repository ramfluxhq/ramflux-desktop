// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! A job in the backup section: its progress while it runs (fetching
//! missing files, what it is working on, bytes and items, cancel until a
//! restore installs), the review of a checked backup before it is
//! installed (what it holds, its file check, and that it merges with the
//! account here, the newer item kept), and the result (counts, file size,
//! the file check, what was kept because it was newer here, per category,
//! or why it failed, with the explicit way past files that could not be
//! fetched).

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::description_list::DescriptionList;
use gpui_kit::component::progress::Progress;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Context, SharedString, Window, div};
use ramflux_desktop_core::{
    AttachmentCheck, BackupFailure, BackupJob, BackupJobKind, BackupPhase, BackupProgress,
};

use super::text::{self, count};
use super::{BackupPanel, running};
use crate::i18n::{self, tr, tr_args};

fn muted(text: SharedString, cx: &App) -> AnyElement {
    div().text_sm().text_color(cx.theme().muted_foreground).child(text).into_any_element()
}

/// "1.2 GiB of 3.4 GiB · 5,120 of 13,983 items", as far as totals are known.
fn amounts(progress: &BackupProgress) -> SharedString {
    let bytes = progress.bytes_total.map(|total| {
        tr_args(
            "settings.backup.job.of",
            &[("done", &i18n::bytes(progress.bytes_done)), ("total", &i18n::bytes(total))],
        )
    });
    let items = progress.items_total.map(|total| {
        tr_args(
            "settings.backup.job.items",
            &[("done", &count(progress.items_done)), ("total", &count(total))],
        )
    });
    let parts: Vec<String> = bytes.into_iter().chain(items).map(|s| s.to_string()).collect();
    SharedString::from(parts.join(" · "))
}

/// A finished restore's file check.
fn file_check(files: &AttachmentCheck) -> Alert {
    let total = files.verified + files.mismatched + files.missing;
    if files.all_verified() {
        Alert::success(
            "restore-files-alert",
            tr_args("settings.backup.done.files_verified", &[("count", &count(files.verified))]),
        )
        .small()
    } else {
        let bad = files.mismatched + files.unapproved_missing();
        Alert::warning(
            "restore-files-alert",
            tr_args(
                "settings.backup.done.files_problem",
                &[("bad", &count(bad)), ("count", &count(total))],
            ),
        )
        .small()
    }
}

/// What a finished restore says about files apart from its check: those
/// recorded as unavailable at the source, and those this device had not
/// downloaded yet.
fn file_notes(files: &AttachmentCheck, cx: &App) -> Vec<AnyElement> {
    let mut notes = Vec::new();
    if files.unavailable_at_source > 0 {
        let note = tr_args(
            "settings.backup.done.unavailable",
            &[("count", &count(files.unavailable_at_source))],
        );
        notes.push(
            div()
                .id("restore-unavailable")
                .test_support()
                .child(muted(note, cx))
                .into_any_element(),
        );
    }
    if files.local_missing_kept > 0 {
        let note = tr_args(
            "settings.backup.done.local_pending",
            &[("count", &count(files.local_missing_kept))],
        );
        notes.push(muted(note, cx));
    }
    notes
}

impl BackupPanel {
    pub(super) fn render_job(&mut self, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(job) = self.displayed(cx) else {
            return div().into_any_element();
        };
        match job.phase {
            phase if running(phase) => self.render_running(&job, cx),
            BackupPhase::Verified => self.render_review(&job, cx),
            _ => self.render_result(&job, cx),
        }
    }

    fn render_running(&self, job: &BackupJob, cx: &mut Context<Self>) -> AnyElement {
        let progress = &job.progress;
        let doing = text::doing(job);
        let fraction = progress.fraction();
        let percent = fraction.map(|f| f * 100.0);
        let label = text::heading(job);
        let cancel = job.phase.is_cancellable().then(|| {
            let id = job.id.clone();
            let key = match job.kind {
                BackupJobKind::Backup => "settings.backup.job.cancel_backup",
                BackupJobKind::Restore => "settings.backup.job.cancel_restore",
            };
            Button::new("backup-job-cancel").small().outline().label(tr(key)).on_click(cx.listener(
                move |this, _, window, cx| {
                    this.cancel(id.clone(), window, cx);
                },
            ))
        });
        v_flex()
            .id("backup-job")
            .test_support()
            .gap_2()
            .child(h_flex().gap_2().justify_between().child(div().text_sm().child(doing)).children(
                percent.map(|p| {
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(SharedString::from(format!("{p:.0}%")))
                }),
            ))
            .child(
                Progress::new("backup-progress")
                    .loading(percent.is_none())
                    .value(percent.unwrap_or(0.0))
                    .accessibility_label(label),
            )
            .child(muted(amounts(progress), cx))
            .child(muted(SharedString::from(job.path.clone()), cx))
            .children(
                (job.phase == BackupPhase::Installing)
                    .then(|| muted(tr("settings.backup.job.installing_note"), cx)),
            )
            .children(cancel.map(|cancel| h_flex().child(cancel)))
            .into_any_element()
    }

    fn render_review(&self, job: &BackupJob, cx: &mut Context<Self>) -> AnyElement {
        let Some(preview) = job.preview.clone() else {
            return muted(tr("settings.backup.job.working"), cx);
        };
        let account =
            preview.account_label.clone().unwrap_or_else(|| preview.account_id.to_string());
        let list = DescriptionList::new()
            .columns(1)
            .item(tr("settings.backup.verified.account"), SharedString::from(account), 1)
            .item(
                tr("settings.backup.verified.made"),
                SharedString::from(i18n::date_time(preview.created_at)),
                1,
            )
            .item(
                tr("settings.backup.size"),
                SharedString::from(i18n::bytes(preview.file_bytes)),
                1,
            );
        let list = text::counts(list, &preview.counts).item(
            tr("settings.backup.verified.files"),
            tr_args(
                "settings.backup.verified.files_value",
                &[("count", &count(preview.attachments.verified))],
            ),
            1,
        );
        let unavailable = preview.attachments.unavailable_at_source;
        let effect = if preview.merge {
            tr("settings.backup.verified.merge")
        } else {
            tr("settings.backup.verified.add")
        };
        let (install, discard) = (job.id.clone(), job.id.clone());
        v_flex()
            .id("backup-review")
            .test_support()
            .gap_3()
            .child(list)
            .children((unavailable > 0).then(|| {
                let note = tr_args(
                    "settings.backup.verified.unavailable",
                    &[("count", &count(unavailable))],
                );
                div()
                    .id("backup-review-unavailable")
                    .test_support()
                    .child(Alert::warning("backup-review-unavailable-alert", note).small())
            }))
            .child(
                div()
                    .id("backup-review-effect")
                    .test_support()
                    .child(Alert::info("backup-review-effect-alert", effect).small()),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("restore-discard")
                            .small()
                            .outline()
                            .label(tr("common.cancel"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.cancel(discard.clone(), window, cx);
                            })),
                    )
                    .child(
                        Button::new("restore-install")
                            .small()
                            .primary()
                            .label(tr("settings.backup.verified.install"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.install(install.clone(), window, cx);
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_result(&self, job: &BackupJob, cx: &mut Context<Self>) -> AnyElement {
        let mut body = v_flex().id("backup-result").test_support().gap_3();
        match (job.kind, job.phase) {
            (BackupJobKind::Backup, BackupPhase::Done) => {
                let size = job.summary.as_ref().map_or(0, |s| s.file_bytes);
                let mut list = DescriptionList::new()
                    .columns(1)
                    .item(tr("settings.backup.file"), SharedString::from(job.path.clone()), 1)
                    .item(tr("settings.backup.size"), SharedString::from(i18n::bytes(size)), 1);
                if let Some(summary) = &job.summary {
                    list = text::counts(list, &summary.counts);
                }
                body = body.child(list);
            }
            (BackupJobKind::Restore, BackupPhase::Done) => {
                let report = job.report.clone().unwrap_or_default();
                let files = report.attachments;
                body = body
                    .child(div().id("restore-files").test_support().child(file_check(&files)))
                    .children(file_notes(&files, cx))
                    .child(text::counts(DescriptionList::new().columns(1), &report.restored));
                let merged = job.preview.as_ref().is_some_and(|p| p.merge);
                if report.kept_newer.items() > 0 {
                    body = body.child(
                        v_flex()
                            .id("restore-kept-newer")
                            .test_support()
                            .gap_1()
                            .child(div().text_sm().child(tr("settings.backup.done.kept_newer")))
                            .child(text::counts(
                                DescriptionList::new().columns(1),
                                &report.kept_newer,
                            )),
                    );
                } else if merged {
                    body = body.child(muted(tr("settings.backup.done.nothing_newer"), cx));
                }
            }
            (kind, BackupPhase::Failed) => {
                let why = job.failure.as_ref().map_or_else(
                    || tr_args("settings.backup.error_other", &[("code", "")]),
                    text::failure,
                );
                body = body.child(
                    div()
                        .id("backup-failure")
                        .test_support()
                        .child(Alert::error("backup-failure-alert", why).small()),
                );
                if kind == BackupJobKind::Restore {
                    body = body.child(muted(tr("settings.backup.failed.unchanged"), cx));
                }
                if let Some(BackupFailure::AttachmentsUnavailable { count: files, .. }) =
                    &job.failure
                {
                    let (job, files) = (job.clone(), *files);
                    body = body.child(muted(tr("settings.backup.missing.help"), cx)).child(
                        h_flex().child(
                            Button::new("backup-allow-missing")
                                .small()
                                .outline()
                                .label(tr("settings.backup.missing.allow"))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.allow_missing(&job, files, window, cx);
                                })),
                        ),
                    );
                }
            }
            (_, _) => {
                body = body.child(muted(tr("settings.backup.cancelled.note"), cx));
            }
        }
        let close = job.id.clone();
        let failed = job.phase == BackupPhase::Failed;
        let retry = failed.then(|| {
            let job = job.clone();
            Button::new("backup-job-retry")
                .small()
                .label(tr("settings.backup.failed.retry"))
                .on_click(cx.listener(move |this, _, window, cx| this.retry(&job, window, cx)))
        });
        body.child(
            h_flex().gap_2().children(retry).child(
                Button::new("backup-job-close")
                    .small()
                    .outline()
                    .label(tr(if failed { "common.close" } else { "settings.backup.close" }))
                    .on_click(cx.listener(move |this, _, _, cx| this.dismiss(close.clone(), cx))),
            ),
        )
        .into_any_element()
    }
}
