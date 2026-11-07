// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The schema gate on screen (S10c c10): the blocking screens, the status
//! line, the reset screen and the backup-import copy. Nothing offered here
//! deletes, converts or retries an account.

mod common;
mod sdk_fake;

use common::*;
use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt as _;
use ramflux_desktop_core::{AccountId, BackupFailure, SchemaGate};
use ramflux_desktop_sdk::SchemaSupport;
use ramflux_desktop_ui::i18n::tr;
use ramflux_desktop_ui::shell::SourceChoice;
use sdk_fake::schema::Generation;
use serde_json::json;

const ACCOUNT: &str = "acct-main";

fn open(
    cx: &mut TestAppContext,
    name: &str,
    support: SchemaSupport,
    setup: impl FnOnce(&sdk_fake::schema::Schema),
) -> (
    ramflux_desktop_test::fake_bus::FakeBus,
    gpui_kit::WindowHandle<gpui_kit::base::Root>,
    gpui_kit::Entity<ramflux_desktop_ui::Shell>,
) {
    let (fake, state, path) = sdk_fake::start(name);
    setup(&state.lock().expect("state").schema);
    let mut options = sdk_fake::options(&path);
    if let SourceChoice::Rfd(config) = &mut options.source {
        config.schema_support = support;
    }
    let (handle, shell) = open_with(cx, options);
    cx.update(|cx| ramflux_desktop_ui::i18n::select_locale("en", cx));
    (fake, handle, shell)
}

fn gate(
    cx: &mut TestAppContext,
    shell: &gpui_kit::Entity<ramflux_desktop_ui::Shell>,
) -> SchemaGate {
    with_store(cx, shell, |s| s.schema_gate(&AccountId::new(ACCOUNT)))
}

fn label_of(
    cx: &mut TestAppContext,
    handle: gpui_kit::WindowHandle<gpui_kit::base::Root>,
    id: &'static str,
) -> Option<String> {
    in_window(cx, handle, move |window, _| {
        window.try_find(id).and_then(|e| e.label().map(str::to_owned))
    })
}

fn settle(
    cx: &mut TestAppContext,
    shell: &gpui_kit::Entity<ramflux_desktop_ui::Shell>,
    want: SchemaGate,
) {
    wait(cx, "the gate verdict", |cx| gate(cx, shell) == want);
    advance(cx, 200);
}

#[gpui_kit::test]
fn blocking_screens_and_the_status_line_follow_the_gate(cx: &mut TestAppContext) {
    let _locale = locale_guard();
    type Row = (&'static str, SchemaSupport, Generation, SchemaGate, &'static str, Option<i64>);
    let rows: [Row; 4] = [
        (
            "older",
            SchemaSupport::V1Candidate,
            Generation::Absent,
            SchemaGate::OlderDaemon,
            "schema.older_daemon.title",
            None,
        ),
        (
            "newer",
            SchemaSupport::Legacy,
            Generation::Is(json!(1)),
            SchemaGate::UpdateApp,
            "schema.update_app.title",
            Some(1),
        ),
        (
            "two",
            SchemaSupport::Legacy,
            Generation::Is(json!(2)),
            SchemaGate::Unsupported,
            "schema.unsupported.title",
            Some(2),
        ),
        (
            "text",
            SchemaSupport::Legacy,
            Generation::Is(json!("1")),
            SchemaGate::Unsupported,
            "schema.unsupported.title",
            None,
        ),
    ];
    for (name, support, generation, want, title, reported) in rows {
        let (fake, handle, shell) =
            open(cx, &format!("s10c-schema-{name}"), support, |s| s.report(generation));
        settle(cx, &shell, want);
        wait(cx, "the blocking screen", |cx| exists(cx, handle, "schema-block"));
        assert_eq!(
            label_of(cx, handle, "schema-block-title"),
            Some(tr(title).to_string()),
            "{name}: the screen says why"
        );
        assert!(!exists(cx, handle, "composer-input"), "{name}: the conversation area is replaced");
        assert!(!exists(cx, handle, "schema-create-account"), "{name}: no reset actions here");
        assert_eq!(
            with_store(cx, &shell, |s| s.schema_generation(&AccountId::new(ACCOUNT))),
            reported,
            "{name}: the reported generation"
        );
        assert!(exists(cx, handle, "status-schema"), "{name}: the account-status schema line");
        fake.shutdown();
    }

    // The daemon's answer is awaited: the last view stays under a status line.
    let (fake, handle, shell) = open(cx, "s10c-schema-held", SchemaSupport::Legacy, |s| {
        s.report(Generation::Is(json!(0)));
        s.hold.close();
    });
    settle(cx, &shell, SchemaGate::Unresolved);
    wait(cx, "the status line", |cx| exists(cx, handle, "schema-banner"));
    assert_eq!(label_of(cx, handle, "schema-banner"), Some(tr("schema.connecting").to_string()));
    assert!(!exists(cx, handle, "schema-block"), "waiting does not block the view");
    fake.shutdown();

    // A status that cannot be read: retrying, under its own line.
    let (fake, handle, shell) = open(cx, "s10c-schema-failed", SchemaSupport::Legacy, |s| {
        s.report(Generation::Fails);
    });
    settle(cx, &shell, SchemaGate::Failed);
    wait(cx, "the failure line", |cx| exists(cx, handle, "schema-banner"));
    assert_eq!(label_of(cx, handle, "schema-banner"), Some(tr("schema.failed").to_string()));
    fake.shutdown();
}

#[gpui_kit::test]
fn schema_reset_and_import_have_no_destructive_action(cx: &mut TestAppContext) {
    let _locale = locale_guard();
    let (fake, handle, shell) = open(cx, "s10c-schema-reset", SchemaSupport::Legacy, |s| {
        s.report(Generation::Is(json!(0)));
        s.old_generation(true);
    });
    settle(cx, &shell, SchemaGate::ResetRequired);
    wait(cx, "the reset screen", |cx| exists(cx, handle, "schema-block"));
    assert_eq!(
        label_of(cx, handle, "schema-block-title"),
        Some(tr("schema.reset.title").to_string())
    );
    assert_eq!(
        label_of(cx, handle, "schema-block-body"),
        Some(tr("schema.reset.body").to_string())
    );
    // The only actions: a new account or another account.
    assert!(exists(cx, handle, "schema-create-account"));
    assert!(exists(cx, handle, "schema-switch-account"));
    for forbidden in
        ["schema-delete", "schema-convert", "schema-retry", "schema-reset", "schema-import"]
    {
        assert!(!exists(cx, handle, forbidden), "{forbidden} is not offered");
    }
    let mark = fake.seen().len();
    click(cx, handle, "schema-switch-account");
    advance(cx, 300);
    let after: Vec<String> = fake.seen()[mark..].iter().map(|r| r.method.clone()).collect();
    for method in
        fake.seen().iter().map(|r| r.method.as_str()).chain(after.iter().map(String::as_str))
    {
        for word in ["delete", "remove", "convert", "migrate", "restore", "erase", "wipe"] {
            assert!(!method.contains(word), "no destructive request on the reset screen: {method}");
        }
    }
    fake.shutdown();

    // The backup import names the same cause and promises the same thing.
    let failure = ramflux_desktop_sdk::translate::backup::failure(
        &json!({ "code": "schema_reset_required" }),
        None,
    );
    assert_eq!(failure, BackupFailure::SchemaResetRequired);
    let copy = tr("backend.backup.schema_reset_required");
    assert!(copy.contains("older development build") && copy.contains("Nothing was changed"));
    assert_eq!(
        ramflux_desktop_sdk::translate::backup::refusal_message("schema_reset_required").as_deref(),
        Some("backend.backup.schema_reset_required")
    );
}
