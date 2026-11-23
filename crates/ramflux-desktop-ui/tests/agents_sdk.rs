// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! AI participants through the production window on the real SDK source
//! (DESK-AGENT-01): the owner's account creates one in the settings section
//! with the same ceremony screens as an account's (no node, no passphrase),
//! sees it listed with its kind and custodian and the note that this app
//! never unlocks it, exports its one-shot credentials to a private file, and
//! reads each refusal as a typed reason. What the person sees is read
//! through the accessibility tree.

mod common;
mod sdk_fake;

use std::sync::Arc;

use common::*;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Entity, TestAppContext, WindowHandle};
use ramflux_desktop_core::{AccountId, AgentExportPhase, methods};
use ramflux_desktop_test::fake_custody::FakeCustody;
use ramflux_desktop_ui::Shell;
use ramflux_desktop_ui::i18n::{agent_export_refusal, tr};
use ramflux_desktop_ui::route::Page;
use ramflux_desktop_ui::settings::SettingsSection;

type Handle = WindowHandle<gpui_kit::base::Root>;

const OWNER: &str = "acct-owner";

/// A daemon with the owner's unlocked account and one AI participant of
/// its own, over `custody`; the settings section is open.
fn open_agents(
    cx: &mut TestAppContext,
    name: &str,
    custody: &Arc<FakeCustody>,
) -> (ramflux_desktop_test::fake_bus::FakeBus, Handle, Entity<Shell>) {
    custody.set_live(true);
    custody.add_account(OWNER, "Me", "owner passphrase", false);
    custody.add_agent("acct-bot", "Bot", OWNER);
    let (fake, state, path) = sdk_fake::start_with_custody(name, custody.clone());
    {
        let mut state = state.lock().expect("state");
        state.born_v3 = true;
        state.accounts = Some(serde_json::json!({ "accounts": [], "account_details": [] }));
    }
    let (handle, shell) = open_sized(cx, sdk_fake::options(&path), 1100.);
    cx.update_window(handle.into(), |_, window, _| window.activate_window()).expect("window");
    wait(cx, "the workspace", |cx| page(cx, &shell) == Page::Workspace);
    wait(cx, "creation offered", |cx| daemon(cx, &shell, |s| s.offers_born_v3_creation()));
    wait(cx, "both listed", |cx| {
        daemon(cx, &shell, |s| s.account(&AccountId::new("acct-bot")).is_some())
    });
    cx.update(|cx| {
        shell.update(cx, |shell, cx| shell.open_settings_at(SettingsSection::Agents, cx))
    });
    wait(cx, "owner event stream", |_| fake.streaming(OWNER));
    cx.run_until_parked();
    in_window(cx, handle, |_, _| {});
    (fake, handle, shell)
}

fn daemon<R>(
    cx: &mut TestAppContext,
    shell: &Entity<Shell>,
    f: impl FnOnce(&ramflux_desktop_core::Store) -> R,
) -> R {
    cx.update(|cx| f(shell.read(cx).store().read(cx).entry_store()))
}

fn export_of(cx: &mut TestAppContext, shell: &Entity<Shell>, id: &str) -> Option<AgentExportPhase> {
    daemon(cx, shell, |s| s.agent_export(&AccountId::new(id)).map(|e| e.phase.clone()))
}

/// Sets the export location input.
fn set_location(cx: &mut TestAppContext, handle: Handle, shell: &Entity<Shell>, value: &str) {
    let panel = cx.update(|cx| shell.read(cx).settings().read(cx).agents_panel().clone());
    let value = value.to_owned();
    in_window(cx, handle, move |window, cx| {
        let input = panel.read(cx).location_input().clone();
        input.update(cx, |input, cx| input.set_value(value, window, cx));
    });
}

#[gpui_kit::test]
fn the_exports_refusals_are_read_as_typed_reasons(cx: &mut TestAppContext) {
    let custody = Arc::new(FakeCustody::new());
    let (fake, handle, shell) = open_agents(cx, "ui-agent-refusals", &custody);
    let dir = std::env::temp_dir().join(format!("rf-ui-agent-refuse-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let taken = dir.join("taken.bundle");
    std::fs::write(&taken, b"mine").expect("file");
    click(cx, handle, "agent-export-acct-bot");

    // A relative location is not sent at all.
    set_location(cx, handle, &shell, "bot.bundle");
    click(cx, handle, "agent-export-submit");
    assert!(has_label(
        cx,
        handle,
        &agent_export_refusal(ramflux_desktop_core::AgentExportRefusal::NoLocation)
    ));
    assert!(fake.seen().iter().all(|s| s.method != "account.export_agent_credentials"));

    // A file that exists is never replaced.
    set_location(cx, handle, &shell, &taken.to_string_lossy());
    click(cx, handle, "agent-export-submit");
    wait(cx, "refused", |cx| export_of(cx, &shell, "acct-bot").is_some());
    let exists_text = agent_export_refusal(ramflux_desktop_core::AgentExportRefusal::FileExists);
    wait(cx, "told", |cx| has_label(cx, handle, &exists_text));
    assert_eq!(std::fs::read(&taken).expect("kept"), b"mine");

    // A folder that does not exist.
    let before = daemon(cx, &shell, |s| s.failures_total());
    set_location(cx, handle, &shell, &dir.join("nope").join("x.bundle").to_string_lossy());
    click(cx, handle, "agent-export-submit");
    wait(cx, "second refusal", |cx| daemon(cx, &shell, |s| s.failures_total()) > before);
    let missing = agent_export_refusal(ramflux_desktop_core::AgentExportRefusal::LocationMissing);
    wait(cx, "told again", |cx| has_label(cx, handle, &missing));

    // Another person's AI participant: the daemon says it is not custodied.
    custody.add_account("acct-other", "Other", "x", false);
    custody.add_agent("acct-theirs", "Theirs", "acct-other");
    fake.publish(
        OWNER,
        "account.state",
        serde_json::json!({"local_account_id":OWNER, "state":"unlocked"}),
    );
    wait(cx, "theirs listed", |cx| {
        daemon(cx, &shell, |s| s.account(&AccountId::new("acct-theirs")).is_some())
    });
    click(cx, handle, "agent-export-acct-theirs");
    set_location(cx, handle, &shell, &dir.join("theirs.bundle").to_string_lossy());
    click(cx, handle, "agent-export-submit");
    let theirs = agent_export_refusal(ramflux_desktop_core::AgentExportRefusal::NotCustodian);
    wait(cx, "not custodian", |cx| has_label(cx, handle, &theirs));
    assert!(!dir.join("theirs.bundle").exists());
    fake.shutdown();
    let _ = std::fs::remove_dir_all(dir);
}

#[gpui_kit::test]
fn the_entry_page_never_offers_to_unlock_an_ai_participant(cx: &mut TestAppContext) {
    let custody = Arc::new(FakeCustody::new());
    custody.set_live(true);
    custody.add_agent("acct-bot", "Bot", OWNER);
    let (fake, state, path) = sdk_fake::start_with_custody("ui-agent-entry", custody.clone());
    {
        let mut state = state.lock().expect("state");
        state.born_v3 = true;
        state.accounts = Some(serde_json::json!({ "accounts": [], "account_details": [] }));
    }
    let (handle, shell) = open_sized(cx, sdk_fake::options(&path), 900.);
    wait(cx, "the AI participant is listed", |cx| {
        daemon(cx, &shell, |s| s.account(&AccountId::new("acct-bot")).is_some())
    });
    wait(cx, "the note", |cx| exists(cx, handle, "account-agent-acct-bot"));
    assert!(has_label(cx, handle, &tr("agent.entry.not_unlocked")));
    assert_eq!(
        in_window(cx, handle, |window, _| window.find("unlock").disabled()),
        Some(true),
        "there is no person's account to unlock"
    );
    fake.shutdown();
}

/// The account slots the window holds (what the switcher lists) and the
/// rendered account.
#[gpui_kit::test]
fn an_unrelated_failure_does_not_reenable_the_export_while_it_is_in_flight(
    cx: &mut TestAppContext,
) {
    let custody = Arc::new(FakeCustody::new());
    let (fake, handle, shell) = open_agents(cx, "ui-agent-double", &custody);
    custody.set_export_delay(std::time::Duration::from_millis(1_500));
    let dir = std::env::temp_dir().join(format!("rf-ui-agent-double-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let path = dir.join("bot.bundle");
    in_window(cx, handle, |window, cx| window.click("agent-export-acct-bot", cx));
    set_location(cx, handle, &shell, &path.to_string_lossy());
    click(cx, handle, "agent-export-submit");
    let sending = |cx: &mut TestAppContext| {
        in_window(cx, handle, |window, _| window.find("agent-export-submit").disabled())
    };
    wait(cx, "sending", |cx| sending(cx) == Some(true));
    // Another effect fails meanwhile (a diagnostics file that cannot be written).
    let before = daemon(cx, &shell, |s| s.failures_total());
    let root = cx.update(|cx| shell.read(cx).root().clone());
    in_window(cx, handle, |window, cx| {
        root.update(cx, |root, cx| {
            root.submit_secret(
                ramflux_desktop_core::IntentKind::ExportDiagnostics {
                    path: "/nonexistent-folder/diagnostics.txt".into(),
                },
                window,
                cx,
            )
        });
    });
    wait(cx, "the other failure", |cx| {
        daemon(cx, &shell, |s| s.failures_total()) > before
            || with_store(cx, &shell, |s| {
                s.failures().iter().any(|f| f.method == methods::DIAGNOSTICS_EXPORT)
            })
    });
    assert_eq!(sending(cx), Some(true), "still waiting for its own answer");
    click(cx, handle, "agent-export-submit");
    assert_eq!(
        fake.seen().iter().filter(|s| s.method == "account.export_agent_credentials").count(),
        1,
        "the second press sent nothing"
    );
    wait(cx, "its own answer", |cx| export_of(cx, &shell, "acct-bot").is_some());
    assert_eq!(export_of(cx, &shell, "acct-bot"), Some(AgentExportPhase::Exported));
    fake.shutdown();
    let _ = std::fs::remove_dir_all(dir);
}

#[gpui_kit::test]
fn the_typed_default_and_the_save_dialog_name_the_same_file_in_the_home_folder(
    cx: &mut TestAppContext,
) {
    let custody = Arc::new(FakeCustody::new());
    let (fake, handle, shell) = open_agents(cx, "ui-agent-default", &custody);
    in_window(cx, handle, |window, cx| window.click("agent-export-acct-bot", cx));
    let panel = cx.update(|cx| shell.read(cx).settings().read(cx).agents_panel().clone());
    let typed = cx.update(|cx| panel.read(cx).location_input().read(cx).value().to_string());
    let home = std::env::var("HOME").expect("home");
    assert_eq!(typed, format!("{home}/ramflux-agent-Bot.bundle"));
    let (name, start) = cx.update(|cx| panel.read(cx).save_dialog_default(cx));
    assert_eq!(name, "ramflux-agent-Bot.bundle");
    assert_eq!(start, std::path::PathBuf::from(&home));
    fake.shutdown();
}
