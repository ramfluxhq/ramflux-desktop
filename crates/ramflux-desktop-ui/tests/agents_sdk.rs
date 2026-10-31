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
use ramflux_desktop_core::{AccountId, AccountKind, AgentExportPhase, CeremonyPhase};
use ramflux_desktop_test::fake_custody::FakeCustody;
use ramflux_desktop_test::mock_ceremony::{DEMO_POSITIONS, demo_word};
use ramflux_desktop_ui::Shell;
use ramflux_desktop_ui::i18n::{agent_custody_problem, agent_export_refusal, tr, tr_args};
use ramflux_desktop_ui::route::Page;
use ramflux_desktop_ui::settings::SettingsSection;
use sdk_fake::wait;

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

fn phase(cx: &mut TestAppContext, shell: &Entity<Shell>) -> Option<CeremonyPhase> {
    daemon(cx, shell, |s| s.ceremony().map(|c| c.phase.clone()))
}

fn held_words(cx: &mut TestAppContext, shell: &Entity<Shell>) -> usize {
    let panel = cx.update(|cx| shell.read(cx).settings().read(cx).agents_panel().clone());
    let ceremony = cx.update(|cx| panel.read(cx).ceremony().clone());
    cx.update(|cx| ceremony.read(cx).held().words())
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

fn through_ceremony(cx: &mut TestAppContext, handle: Handle, shell: &Entity<Shell>, name: &str) {
    click(cx, handle, "agent-create");
    type_into(cx, handle, "ceremony-name-input", name);
    click(cx, handle, "ceremony-continue");
    wait(cx, "prepared", |cx| phase(cx, shell) == Some(CeremonyPhase::Prepared));
    // The same safeguards as an account's: the warning, then the words, once.
    assert!(has_label(cx, handle, &tr("ceremony.ready.warning")));
    click(cx, handle, "ceremony-show");
    wait(cx, "shown", |cx| phase(cx, shell) == Some(CeremonyPhase::PhraseReady));
    wait(cx, "taken", |cx| held_words(cx, shell) == 24);
    assert!(has_label(cx, handle, &tr("ceremony.show.warning")));
    click(cx, handle, "ceremony-hide");
    assert_eq!(held_words(cx, shell), 0, "erased before the daemon is asked");
    wait(cx, "asked", |cx| matches!(phase(cx, shell), Some(CeremonyPhase::Challenged { .. })));
    for (id, position) in
        ["ceremony-word-1", "ceremony-word-2", "ceremony-word-3"].into_iter().zip(DEMO_POSITIONS)
    {
        type_into(cx, handle, id, demo_word(usize::from(position)));
    }
}

#[gpui_kit::test]
fn an_ai_participant_is_created_listed_and_its_credentials_exported_once(cx: &mut TestAppContext) {
    let custody = Arc::new(FakeCustody::new());
    let (fake, handle, shell) = open_agents(cx, "ui-agent-flow", &custody);

    // The list shows the existing AI participant with its kind, the note
    // that this app never unlocks it, and no custodian the daemon did not name.
    assert!(exists(cx, handle, "agent-acct-bot"));
    let detail = tr_args(
        "agent.row.detail",
        &[
            ("kind", &ramflux_desktop_ui::i18n::account_kind(AccountKind::HeadlessAi)),
            ("custodian", &tr("agent.row.custodian_unknown")),
        ],
    );
    assert!(has_label(cx, handle, &detail), "{detail}");
    assert!(has_label(cx, handle, &tr("agent.row.not_unlocked")));

    // Creation: no node line, no passphrase screen, three words confirm it.
    click(cx, handle, "agent-create");
    assert!(has_label(cx, handle, &tr_args("agent.create.custodian", &[("name", "Me")])));
    assert!(!exists(cx, handle, "ceremony-node"), "no node is asked for");
    click(cx, handle, "ceremony-cancel");
    through_ceremony(cx, handle, &shell, "Ledger");
    click(cx, handle, "ceremony-next");
    wait(cx, "created", |cx| matches!(phase(cx, &shell), Some(CeremonyPhase::Created(_))));
    assert!(!exists(cx, handle, "ceremony-passphrase-input"), "the daemon makes the passphrase");
    assert_eq!(custody.last_confirm_fields(), vec!["words".to_owned()]);
    let Some(CeremonyPhase::Created(made)) = phase(cx, &shell) else { panic!("created") };
    let id = made.local_account_id.clone();
    assert!(has_label(cx, handle, &tr_args("agent.done.text", &[("name", "Ledger")])));
    assert!(has_label(cx, handle, &tr("agent.done.next")));
    let begin =
        fake.seen().into_iter().find(|s| s.method == "account.create.begin").expect("begin");
    assert_eq!(begin.account.as_deref(), Some(OWNER));
    assert_eq!(begin.body["principal_kind"], "headless_ai");
    assert_eq!(begin.body["custodian_account"], OWNER);
    click(cx, handle, "ceremony-done-close");
    // It is listed as an AI participant the owner custodies.
    wait(cx, "listed", |cx| exists(cx, handle, &format!("agent-{id}")));
    assert_eq!(
        daemon(cx, &shell, |s| s.custodian_of(&AccountId::new(&id)).cloned()),
        Some(AccountId::new(OWNER))
    );
    let line = tr_args(
        "agent.row.detail",
        &[
            ("kind", &ramflux_desktop_ui::i18n::account_kind(AccountKind::HeadlessAi)),
            ("custodian", &tr_args("agent.row.custodian", &[("name", "Me")])),
        ],
    );
    assert!(has_label(cx, handle, &line), "{line}");

    // Export it: a location in the owner's home, written once.
    let export_button = format!("agent-export-{id}");
    in_window(cx, handle, move |window, cx| window.click(export_button, cx));
    assert!(has_label(cx, handle, &tr("agent.export.once")));
    let dir = std::env::temp_dir().join(format!("rf-ui-agent-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let path = dir.join("ledger.bundle");
    set_location(cx, handle, &shell, &path.to_string_lossy());
    click(cx, handle, "agent-export-submit");
    wait(cx, "exported", |cx| export_of(cx, &shell, &id) == Some(AgentExportPhase::Exported));
    wait(cx, "shown", |cx| {
        has_label(cx, handle, &tr_args("agent.export.done", &[("path", &path.to_string_lossy())]))
    });
    assert!(has_label(cx, handle, &tr("agent.export.done_next")));
    use std::os::unix::fs::PermissionsExt as _;
    assert_eq!(std::fs::metadata(&path).expect("file").permissions().mode() & 0o777, 0o600);
    // One-shot: the control is off, and nothing on screen is a credential.
    assert_eq!(
        in_window(cx, handle, |window, _| window.find("agent-export-submit").disabled()),
        Some(true)
    );
    assert!(!has_label(cx, handle, "daemon-made"));
    let sent = fake
        .seen()
        .into_iter()
        .find(|s| s.method == "account.export_agent_credentials")
        .expect("export");
    assert_eq!(sent.account.as_deref(), Some(OWNER));
    assert_eq!(sent.body["account"], id.as_str());
    fake.shutdown();
    let _ = std::fs::remove_dir_all(dir);
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
fn a_custodian_the_daemon_refuses_ends_the_ceremony_with_its_own_reason(cx: &mut TestAppContext) {
    let custody = Arc::new(FakeCustody::new());
    let (fake, handle, shell) = open_agents(cx, "ui-agent-custodian", &custody);
    // The daemon refuses something the window cannot see.
    custody.refuse_headless(true);
    click(cx, handle, "agent-create");
    type_into(cx, handle, "ceremony-name-input", "Ledger");
    click(cx, handle, "ceremony-continue");
    wait(cx, "refused", |cx| {
        matches!(
            phase(cx, &shell),
            Some(CeremonyPhase::Failed(ramflux_desktop_core::CeremonyFailure::CustodianRefused))
        )
    });
    assert!(has_label(cx, handle, &tr("agent.ended.custodian_refused")));
    assert_eq!(custody.running(), 0);
    fake.shutdown();
}

#[gpui_kit::test]
fn an_account_that_cannot_custody_is_told_why_before_anything_is_sent(cx: &mut TestAppContext) {
    let custody = Arc::new(FakeCustody::new());
    let (fake, handle, shell) = open_agents(cx, "ui-agent-preflight", &custody);
    // The owner's account loses its connection: the control is off with the
    // reason, and no request goes out.
    custody.set_networking_of(OWNER, false);
    let reason =
        agent_custody_problem(Some(ramflux_desktop_core::AgentCustodyProblem::NotConnected));
    wait(cx, "the reason", |cx| has_label(cx, handle, &reason));
    assert_eq!(
        in_window(cx, handle, |window, _| window.find("agent-create").disabled()),
        Some(true)
    );
    assert!(fake.seen().iter().all(|s| s.method != "account.create.begin"));
    let _ = shell;
    fake.shutdown();
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
