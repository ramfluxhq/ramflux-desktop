// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The production window on a fake daemon with the event stream: a
//! conversation someone else starts appears through `conversation.changed`
//! with nothing polled, Settings › Diagnostics says the account is live
//! from the stream (and polls once the daemon refuses it), and the polling
//! fallback switch is saved with the launch settings.

mod common;
mod sdk_fake;

use common::*;
use gpui_kit::TestAppContext;
use ramflux_desktop_core::{ContextId, ParticipantId, SyncMode};
use ramflux_desktop_ui::route::Page;
use ramflux_desktop_ui::settings::SettingsSection;
use sdk_fake::wait;
use serde_json::json;

const ACCOUNT: &str = "acct-main";

fn mode(
    cx: &mut TestAppContext,
    shell: &gpui_kit::Entity<ramflux_desktop_ui::Shell>,
) -> Option<SyncMode> {
    with_store(cx, shell, |s| s.sync_status().map(|sync| sync.mode))
}

#[gpui_kit::test]
fn the_window_follows_the_stream_and_diagnostics_says_so(cx: &mut TestAppContext) {
    let (fake, state, path) = sdk_fake::start("ui-sync-stream");
    fake.enable_stream();
    // Tall enough to show the message sync group below the local daemon.
    let (handle, shell) = open_sized(cx, sdk_fake::options(&path), 2400.);
    wait(cx, "workspace", |cx| page(cx, &shell) == Page::Workspace);
    wait(cx, "the stream", |cx| mode(cx, &shell) == Some(SyncMode::Stream));
    wait(cx, "contacts", |cx| {
        with_store(cx, &shell, |s| s.participant(&ParticipantId::new("principal-bo")).is_some())
    });

    // Zoe starts a conversation: the daemon lists it and says so.
    let mark = fake.seen().len();
    let mut row = sdk_fake::frames()["conversation.directory"]["conversations"][0].clone();
    row["id"] = json!("dm-from-zoe");
    row["participants"] = json!(["principal-me", "principal-zoe"]);
    state.lock().expect("state").conversations.push(row.clone());
    fake.publish(ACCOUNT, "conversation.changed", row);
    let new = ContextId::new("dm-from-zoe");
    wait(cx, "the new conversation", |cx| with_store(cx, &shell, |s| s.context(&new).is_some()));
    let pulled = fake.seen().into_iter().skip(mark).any(|s| s.method == "message.receive");
    assert!(!pulled, "nothing is pulled from the node by the window");

    cx.update(|cx| {
        shell.update(cx, |shell, cx| shell.open_settings_at(SettingsSection::Diagnostics, cx));
    });
    cx.run_until_parked();
    assert!(exists(cx, handle, "diagnostics-sync"));
    assert!(has_label(
        cx,
        handle,
        &ramflux_desktop_ui::i18n::tr("settings.diagnostics.sync_mode.stream")
    ));

    // The switch saves the fallback choice.
    let launcher = cx.update(|cx| shell.read(cx).launcher().clone());
    click(cx, handle, "diagnostics-poll-fallback");
    let saved = cx.update(|cx| launcher.read(cx).settings().poll_fallback);
    assert_eq!(saved, Some(false), "turned off and saved");

    // The daemon stops offering the stream: the account polls and says so.
    fake.disable_stream();
    fake.end_streams(ACCOUNT, false);
    wait(cx, "polling", |cx| mode(cx, &shell) == Some(SyncMode::Polling));
    in_window(cx, handle, |_, _| {});
    assert!(has_label(
        cx,
        handle,
        &ramflux_desktop_ui::i18n::tr("settings.diagnostics.sync_mode.polling")
    ));
    fake.shutdown();
}
