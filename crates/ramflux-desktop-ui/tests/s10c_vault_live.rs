// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! S10c c09 r2: the nine key-vault refusals through the production views.
//! Each literal error frame (`vault_rows`, bytes as the daemon writes them)
//! is answered by the fake daemon on the real SDK source, travels bus ->
//! bridge -> event -> store, and is read where a person reads it: the
//! ceremony and recovery panels, the status strip after a refused send, and
//! the AI participant export view (panel and strip), in both languages.
//! The interface language is process-wide: the tests hold the locale guard
//! and end in English. One test per reason and path, each with a context and
//! daemon of its own (a test that opens many windows leaves their bridges
//! reconnecting to daemons that are gone).

mod common;
mod sdk_fake;
mod vault_rows;

use std::sync::Arc;

use common::*;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Entity, TestAppContext, WindowHandle};
use ramflux_desktop_core::{
    AccountId, AgentExportPhase, CeremonyPhase, ContextId, FailureCode, RecoveryPhase,
};
use ramflux_desktop_test::fake_custody::FakeCustody;
use ramflux_desktop_ui::Shell;
use ramflux_desktop_ui::route::{EntryStep, Page};
use ramflux_desktop_ui::settings::SettingsSection;
use vault_rows::{ROWS, Row};

type Handle = WindowHandle<gpui_kit::base::Root>;
type Fake = ramflux_desktop_test::fake_bus::FakeBus;

const LOCALES: [&str; 2] = ["en", "zh-CN"];

/// A hang guard for the steps that are not condition waits (those panic at
/// `WAIT_LIMIT`): if a step is not finished within the limit, the process
/// reports which one and exits, so a hang fails fast with its name instead
/// of running into the CI step timeout.
struct Guard {
    done: Arc<std::sync::atomic::AtomicBool>,
}

impl Guard {
    const LIMIT: std::time::Duration = std::time::Duration::from_secs(45);

    fn new(step: String) -> Self {
        let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = done.clone();
        std::thread::spawn(move || {
            let until = std::time::Instant::now() + Self::LIMIT;
            while std::time::Instant::now() < until {
                if flag.load(std::sync::atomic::Ordering::SeqCst) {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            eprintln!("HANG GUARD: step `{step}` did not finish in {:?}", Self::LIMIT);
            std::process::exit(101);
        });
        Self { done }
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.done.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

fn frame_of(row: &Row) -> serde_json::Value {
    serde_json::from_str(row.frame).expect("frame json")
}

/// The label of an element, when it is shown.
fn label_of(cx: &mut TestAppContext, handle: Handle, id: &'static str) -> Option<String> {
    in_window(cx, handle, move |window, _| {
        window.try_find(id).and_then(|element| element.label().map(str::to_owned))
    })
}

/// Switches the interface language the way the settings do.
fn speak(cx: &mut TestAppContext, locale: &str) {
    cx.update(|cx| ramflux_desktop_ui::i18n::select_locale(locale, cx));
    cx.run_until_parked();
}

/// The daemon's own store (the rendered one is the opened account's once
/// there is one).
fn daemon<R>(
    cx: &mut TestAppContext,
    shell: &Entity<Shell>,
    f: impl FnOnce(&ramflux_desktop_core::Store) -> R,
) -> R {
    cx.update(|cx| f(shell.read(cx).store().read(cx).entry_store()))
}

/// A fresh device whose daemon answers every begin with `row`'s frame; one
/// locked account is there to recover.
fn fresh_device(
    cx: &mut TestAppContext,
    name: &str,
    row: &Row,
    locked_account: bool,
) -> (Fake, Handle, Entity<Shell>) {
    let custody = Arc::new(FakeCustody::new());
    if locked_account {
        custody.add_locked_account("acct-r", "Alex", "river stone lantern");
    }
    custody.set_error_frame(Some(frame_of(row)));
    let (fake, state, path) = sdk_fake::start_with_custody(name, custody);
    {
        let mut state = state.lock().expect("state");
        state.born_v3 = true;
        state.no_default = true;
        state.accounts = Some(serde_json::json!({ "accounts": [], "account_details": [] }));
    }
    let (handle, shell) = open_sized(cx, sdk_fake::options(&path), 900.);
    cx.update_window(handle.into(), |_, window, _| window.activate_window()).expect("window");
    wait(cx, "the entry page", |cx| matches!(page(cx, &shell), Page::Entry(_)));
    if page(cx, &shell) == Page::Entry(EntryStep::Node) {
        click_in(cx, handle, "node-n-harbor", "node-confirm");
    }
    wait(cx, "the account step", |cx| page(cx, &shell) == Page::Entry(EntryStep::Account));
    wait(cx, "creation offered", |cx| daemon(cx, &shell, |s| s.offers_born_v3_creation()));
    if locked_account {
        wait(cx, "the account is listed", |cx| {
            daemon(cx, &shell, |s| s.account(&AccountId::new("acct-r")).is_some())
        });
    }
    (fake, handle, shell)
}

fn ceremony_row(cx: &mut TestAppContext, ix: usize) {
    let _locale = locale_guard();
    let row = &ROWS[ix];
    let name = row.reason_code;
    let _hang = Guard::new(format!("creation row: {name}"));
    speak(cx, "en");
    let (fake, handle, shell) = fresh_device(cx, &format!("ui-vault-c{ix}"), row, false);
    click(cx, handle, "account-create-phrase");
    type_into(cx, handle, "ceremony-name-input", "Alex");
    click(cx, handle, "ceremony-continue");
    wait(cx, name, |cx| {
        daemon(cx, &shell, |s| s.ceremony().map(|c| c.phase.clone()))
            == Some(CeremonyPhase::Failed(row.custody_failure))
    });
    for locale in LOCALES {
        speak(cx, locale);
        assert_eq!(
            label_of(cx, handle, "ceremony-ended-text").as_deref(),
            Some(row.ceremony.in_locale(locale)),
            "{name} {locale}: the creation that ended"
        );
    }
    assert!(fake.seen().iter().any(|s| s.method.starts_with("account.create")), "{name}");
    speak(cx, "en");
    fake.shutdown();
}

fn recovery_row(cx: &mut TestAppContext, ix: usize) {
    let _locale = locale_guard();
    let row = &ROWS[ix];
    let name = row.reason_code;
    let _hang = Guard::new(format!("recovery row: {name}"));
    speak(cx, "en");
    let (fake, handle, shell) = fresh_device(cx, &format!("ui-vault-r{ix}"), row, true);
    click(cx, handle, "account-recover-acct-r");
    click(cx, handle, "recovery-continue");
    wait(cx, name, |cx| {
        daemon(cx, &shell, |s| s.recovery().map(|r| r.phase.clone()))
            == Some(RecoveryPhase::Failed(row.custody_failure))
    });
    for locale in LOCALES {
        speak(cx, locale);
        assert_eq!(
            label_of(cx, handle, "recovery-ended-text").as_deref(),
            Some(row.recovery.in_locale(locale)),
            "{name} {locale}: the recovery that ended"
        );
    }
    assert!(fake.seen().iter().any(|s| s.method.starts_with("account.recover")), "{name}");
    speak(cx, "en");
    fake.shutdown();
}

fn send_row(cx: &mut TestAppContext, ix: usize) {
    let _locale = locale_guard();
    let row = &ROWS[ix];
    let name = row.reason_code;
    let _hang = Guard::new(format!("refused send row: {name}"));
    speak(cx, "en");
    let (fake, state, path) = sdk_fake::start(&format!("ui-vault-b{ix}"));
    state.lock().expect("state").fail_submit_frame = Some(frame_of(row));
    let (handle, shell) = open_with(cx, sdk_fake::options(&path));
    wait(cx, "workspace", |cx| page(cx, &shell) == Page::Workspace);
    wait(cx, "the conversation", |cx| {
        with_store(cx, &shell, |s| s.context(&ContextId::new("dm-ana")).is_some())
    });
    show_context(cx, handle, &shell, "dm-ana");
    type_into(cx, handle, "composer-input", "hello");
    click(cx, handle, "composer-send");
    wait(cx, name, |cx| with_store(cx, &shell, |s| !s.failures().is_empty()));
    let code = with_store(cx, &shell, |s| s.failures().last().map(|f| f.code.clone()));
    assert_eq!(code, Some(FailureCode::Daemon(row.wire_code.to_owned())), "{name}: code");
    for (locale, action) in [("en", "Message: "), ("zh-CN", "\u{6d88}\u{606f}\u{ff1a}")] {
        speak(cx, locale);
        let wanted = format!("{action}{}", row.bus.in_locale(locale));
        assert_eq!(
            label_of(cx, handle, "status-failure-value").as_deref(),
            Some(wanted.as_str()),
            "{name} {locale}: the refused send in the strip"
        );
    }
    let submits = fake.seen().iter().filter(|s| s.method == "message.submit").count();
    assert_eq!(submits, 1, "{name}: the refusal was not retried by the desktop");
    speak(cx, "en");
    fake.shutdown();
}

fn export_row(cx: &mut TestAppContext, ix: usize) {
    let _locale = locale_guard();
    let row = &ROWS[ix];
    let name = row.reason_code;
    let _hang = Guard::new(format!("agent export row: {name}"));
    speak(cx, "en");
    let custody = Arc::new(FakeCustody::new());
    custody.set_live(true);
    custody.add_account("acct-owner", "Me", "owner passphrase", false);
    custody.add_agent("acct-bot", "Bot", "acct-owner");
    custody.set_error_frame(Some(frame_of(row)));
    let (fake, state, path) =
        sdk_fake::start_with_custody(&format!("ui-vault-a{ix}"), custody.clone());
    {
        let mut state = state.lock().expect("state");
        state.born_v3 = true;
        state.accounts = Some(serde_json::json!({ "accounts": [], "account_details": [] }));
    }
    let (handle, shell) = open_sized(cx, sdk_fake::options(&path), 1100.);
    cx.update_window(handle.into(), |_, window, _| window.activate_window()).expect("window");
    wait(cx, "the workspace", |cx| page(cx, &shell) == Page::Workspace);
    wait(cx, "both listed", |cx| {
        daemon(cx, &shell, |s| s.account(&AccountId::new("acct-bot")).is_some())
    });
    cx.update(|cx| {
        shell.update(cx, |shell, cx| shell.open_settings_at(SettingsSection::Agents, cx))
    });
    wait(cx, "owner event stream", |_| fake.streaming("acct-owner"));
    cx.run_until_parked();
    in_window(cx, handle, |_, _| {});
    click(cx, handle, "agent-export-acct-bot");
    let dir = std::env::temp_dir().join(format!("rf-ui-vault-export-{}-{ix}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let panel = cx.update(|cx| shell.read(cx).settings().read(cx).agents_panel().clone());
    let target = dir.join("bot.bundle").to_string_lossy().into_owned();
    in_window(cx, handle, move |window, cx| {
        let input = panel.read(cx).location_input().clone();
        input.update(cx, |input, cx| input.set_value(target, window, cx));
    });
    click(cx, handle, "agent-export-submit");
    wait(cx, name, |cx| daemon(cx, &shell, |s| s.failures_total()) > 0);
    let phase = daemon(cx, &shell, |s| {
        s.agent_export(&AccountId::new("acct-bot")).map(|e| e.phase.clone())
    });
    assert_eq!(phase, Some(AgentExportPhase::Refused(row.export_refusal)), "{name}");
    // The failed effect the store keeps says the same in words.
    let failed = daemon(cx, &shell, |s| s.failures().last().cloned()).expect("a failure");
    assert_eq!(failed.code, FailureCode::Daemon(row.wire_code.to_owned()), "{name}");
    for locale in LOCALES {
        speak(cx, locale);
        assert!(
            has_label(cx, handle, row.export.in_locale(locale)),
            "{name} {locale}: the export view says {}",
            row.export.in_locale(locale)
        );
        assert_eq!(
            ramflux_desktop_ui::i18n::backend_text(&failed.message).to_string(),
            row.export_backend.in_locale(locale),
            "{name} {locale}: the failed effect"
        );
    }
    speak(cx, "en");
    // Nothing was written, whatever the reason.
    assert!(std::fs::read_dir(&dir).expect("dir").next().is_none());
    assert!(custody.exports().is_empty());
    fake.shutdown();
    let _ = std::fs::remove_dir_all(dir);
}

/// One test per reason on each path.
macro_rules! rows {
    ($($ix:literal $name:ident),* $(,)?) => {
        mod creation {
            use super::*;
            $(#[gpui_kit::test] fn $name(cx: &mut TestAppContext) { ceremony_row(cx, $ix) })*
        }
        mod recovery {
            use super::*;
            $(#[gpui_kit::test] fn $name(cx: &mut TestAppContext) { recovery_row(cx, $ix) })*
        }
        mod refused_send {
            use super::*;
            $(#[gpui_kit::test] fn $name(cx: &mut TestAppContext) { send_row(cx, $ix) })*
        }
        mod agent_export {
            use super::*;
            $(#[gpui_kit::test] fn $name(cx: &mut TestAppContext) { export_row(cx, $ix) })*
        }
    };
}

rows!(
    0 vault_missing,
    1 vault_init_pending,
    2 vault_import_pending,
    3 vault_corrupt,
    4 vault_permission,
    5 vault_locked,
    6 vault_busy,
    7 vault_primary_unavailable,
    8 vault_mode_mismatch,
);
