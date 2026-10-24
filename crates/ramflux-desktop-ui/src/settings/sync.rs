// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Settings · Diagnostics, message sync: how the shown account is kept up
//! to date (the daemon's event stream, polling as the fallback, or not at
//! all), why the stream is not used when it is not, the daemon's own
//! background sync, and the one setting that allows the polling fallback.
//!
//! Everything shown comes from the store's sync status, which holds states,
//! times, counts and a stream position, never content.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::description_list::DescriptionList;
use gpui_kit::component::setting::{SettingGroup, SettingItem};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, Role, SharedString, div};
use ramflux_desktop_core::{DaemonSyncState, SyncMode, SyncStatus};

use super::SectionContext;
use crate::i18n::{date_time, tr, tr_args};

/// The locale key naming `mode`.
pub fn mode_key(mode: SyncMode) -> &'static str {
    match mode {
        SyncMode::Connecting => "settings.diagnostics.sync_mode.connecting",
        SyncMode::Stream => "settings.diagnostics.sync_mode.stream",
        SyncMode::Polling => "settings.diagnostics.sync_mode.polling",
        SyncMode::Off => "settings.diagnostics.sync_mode.off",
    }
}

fn daemon_key(state: DaemonSyncState) -> &'static str {
    match state {
        DaemonSyncState::Running => "settings.diagnostics.daemon_sync.running",
        DaemonSyncState::Idle => "settings.diagnostics.daemon_sync.idle",
        DaemonSyncState::Error => "settings.diagnostics.daemon_sync.error",
    }
}

/// The label and value rows shown for `status`.
pub fn rows(status: &SyncStatus) -> Vec<(&'static str, SharedString)> {
    let mut rows = vec![("settings.diagnostics.sync_updates", tr(mode_key(status.mode)))];
    if let Some(reason) = &status.reason {
        rows.push(("settings.diagnostics.sync_reason", SharedString::from(reason.clone())));
    }
    if let Some(state) = status.daemon_sync {
        let value = match status.last_run {
            Some(at) => tr_args(
                "settings.diagnostics.daemon_sync.with_time",
                &[("state", &tr(daemon_key(state))), ("time", &date_time(at))],
            ),
            None => tr(daemon_key(state)),
        };
        rows.push(("settings.diagnostics.daemon_sync.label", value));
    }
    if let Some(waiting) = status.unrouted.filter(|n| *n > 0) {
        rows.push(("settings.diagnostics.sync_unrouted", SharedString::from(waiting.to_string())));
    }
    if let Some(cursor) = status.cursor {
        rows.push(("settings.diagnostics.sync_cursor", SharedString::from(cursor.to_string())));
    }
    rows
}

/// The message sync group, when an account is shown (its source reports
/// how it is kept up to date).
pub fn group(context: &SectionContext, cx: &App) -> Option<SettingGroup> {
    let status = context.store.read(cx).store().sync_status().cloned()?;
    let fallback = context.launcher.read(cx).settings().poll_fallback_or_default();
    let launcher = context.launcher.clone();
    let mode = tr(mode_key(status.mode));
    let rows = rows(&status);
    let item = SettingItem::render(move |_, _, cx: &mut App| {
        let muted = cx.theme().muted_foreground;
        let list = rows.iter().fold(DescriptionList::new().columns(1), |list, (key, value)| {
            list.item(tr(key), value.clone(), 1)
        });
        let launcher = launcher.clone();
        v_flex()
            .gap_3()
            .child(
                div()
                    .id("diagnostics-sync")
                    .role(Role::Status)
                    .aria_label(mode.clone())
                    .test_support()
                    .child(list),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        Switch::new("diagnostics-poll-fallback")
                            .label(tr("settings.diagnostics.poll_fallback"))
                            .checked(fallback)
                            .on_change(move |checked, _, cx| {
                                let on = *checked;
                                launcher.update(cx, |l, cx| l.set_poll_fallback(on, cx));
                            }),
                    )
                    .child(
                        div()
                            .id("diagnostics-poll-fallback-help")
                            .text_xs()
                            .text_color(muted)
                            .child(tr("settings.diagnostics.poll_fallback_help"))
                            .test_support(),
                    ),
            )
            .into_any_element()
    });
    Some(SettingGroup::new().title(tr("settings.diagnostics.sync")).item(item))
}
