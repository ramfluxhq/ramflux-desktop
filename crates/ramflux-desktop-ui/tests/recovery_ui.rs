// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Recovering an account from its recovery phrase (born-v3), in the
//! production window on the mock source, read through the accessibility
//! tree: the entry is on a locked person's account of a daemon that offers
//! it, and nowhere else; it opens only from the user's own action and sends
//! nothing secret before the account fingerprint is typed; the 24 words and
//! the passphrase leave their inputs the moment they are sent; the five
//! minutes are visible and a record nobody asked for is never shown; cancel
//! is always there; initial focus never lands on a button.

mod common;

use common::*;
use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{AppContext as _, Entity, Role, TestAppContext, WindowHandle};
use ramflux_desktop_core::{
    Account, AccountId, AccountKind, CEREMONY_TTL_MS, CeremonyFailure, DomainEvent, LockState,
    Networking, RecoveryCeremony, RecoveryPhase, SecureSessions, Timestamp,
};
use ramflux_desktop_test::mock_ceremony::demo_word;
use ramflux_desktop_test::mock_recovery::{DEMO_ANCHOR, demo_phrase};
use ramflux_desktop_ui::Shell;
use ramflux_desktop_ui::entry::{RecoveryPanel, RecoveryPending};
use ramflux_desktop_ui::i18n::{tr, tr_args};
use ramflux_desktop_ui::route::{EntryStep, Page};

type Handle = WindowHandle<gpui_kit::base::Root>;

const PASSPHRASE: &str = "river stone lantern";
const ID: &str = "acct-r";

fn account(id: &str, kind: AccountKind, lock_state: LockState) -> DomainEvent {
    DomainEvent::AccountUpdated(Account {
        id: AccountId::new(id),
        label: "Alex".to_owned(),
        kind,
        node_id: None,
        device_id: None,
        lock_state,
        self_participant_id: None,
        secure_sessions: SecureSessions::Ready,
        networking: Networking::Live,
    })
}

/// Runs `f` on the mock daemon of the window.
fn mock<R>(
    cx: &mut TestAppContext,
    shell: &Entity<Shell>,
    f: impl FnOnce(&mut ramflux_desktop_test::MockSource) -> R,
) -> R {
    let handle = store_handle(cx, shell);
    cx.update(|cx| handle.update(cx, |handle, _| f(handle.mock_mut().expect("a mock"))))
}

/// A device with one locked person's account on a daemon that offers the
/// phrase, and the account's passphrase known to the daemon.
fn world(cx: &mut TestAppContext) -> (Handle, Entity<Shell>) {
    let (handle, shell) = open(cx, "s13_born_v3");
    cx.update_window(handle.into(), |_, window, _| window.activate_window()).expect("window");
    cx.run_until_parked();
    advance(cx, 2_500);
    mock(cx, &shell, |m| {
        m.ceremony_mut().recovery_mut().allow(ID, "Alex", PASSPHRASE);
        m.inject(0, account(ID, AccountKind::HumanInteractive, LockState::Locked));
    });
    advance(cx, 300);
    assert_eq!(page(cx, &shell), Page::Entry(EntryStep::Account));
    (handle, shell)
}

fn panel(cx: &mut TestAppContext, shell: &Entity<Shell>) -> Entity<RecoveryPanel> {
    cx.update(|cx| shell.read(cx).entry().read(cx).recovery().clone())
}

fn record(cx: &mut TestAppContext, shell: &Entity<Shell>) -> Option<RecoveryCeremony> {
    with_store(cx, shell, |s| s.recovery().cloned())
}

fn phase(cx: &mut TestAppContext, shell: &Entity<Shell>) -> Option<RecoveryPhase> {
    record(cx, shell).map(|r| r.phase)
}

fn count(cx: &mut TestAppContext, shell: &Entity<Shell>, label: &str) -> usize {
    effects(cx, shell).iter().filter(|l| l.starts_with(label)).count()
}

fn disabled(cx: &mut TestAppContext, handle: Handle, id: &'static str) -> Option<bool> {
    in_window(cx, handle, move |window, _| window.find(id).disabled())
}

fn pending(cx: &mut TestAppContext, shell: &Entity<Shell>) -> Option<RecoveryPending> {
    let panel = panel(cx, shell);
    cx.update(|cx| panel.read(cx).pending())
}

fn target(cx: &mut TestAppContext, handle: Handle, shell: &Entity<Shell>) -> Option<&'static str> {
    let panel = panel(cx, shell);
    in_window(cx, handle, |_, _| {});
    cx.update_window(handle.into(), move |_, window, cx| panel.read(cx).focus_target(window, cx))
        .expect("window")
}

/// Whether every input of the panel is empty.
fn inputs_empty(cx: &mut TestAppContext, shell: &Entity<Shell>) -> bool {
    let panel = panel(cx, shell);
    cx.update(|cx| panel.read(cx).inputs().into_iter().all(|i| i.read(cx).value().is_empty()))
}

/// The fingerprint as the screen groups it.
fn grouped() -> String {
    DEMO_ANCHOR
        .chars()
        .collect::<Vec<_>>()
        .chunks(4)
        .map(|g| g.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Moves the store's clock (and so the recovery's) forward at once.
fn jump(cx: &mut TestAppContext, shell: &Entity<Shell>, ms: u64) {
    let handle = store_handle(cx, shell);
    cx.update(|cx| handle.update(cx, |handle, cx| handle.advance(ms, cx)));
    advance(cx, 100);
}

/// From the account list to the first screen.
fn open_recovery(cx: &mut TestAppContext, handle: Handle) {
    click(cx, handle, "account-recover-acct-r");
}

/// To the screen that asks for the fingerprint, the words and the passphrase.
fn to_check(cx: &mut TestAppContext, handle: Handle) {
    open_recovery(cx, handle);
    click(cx, handle, "recovery-continue");
    advance(cx, 300);
}

fn type_all(cx: &mut TestAppContext, handle: Handle, fingerprint: &str, phrase: &str, pass: &str) {
    type_into(cx, handle, "recovery-fingerprint-input", fingerprint);
    type_into(cx, handle, "recovery-phrase-input", phrase);
    type_into(cx, handle, "recovery-passphrase-input", pass);
}

#[gpui_kit::test]
fn the_entry_is_on_a_locked_persons_account_of_a_daemon_that_offers_the_phrase(
    cx: &mut TestAppContext,
) {
    let (handle, shell) = world(cx);
    assert!(exists(cx, handle, "account-recover-acct-r"));
    assert!(has_label(cx, handle, &tr_args("recovery.entry.label", &[("name", "Alex")])));
    // An unlocked account has nothing to recover.
    mock(cx, &shell, |m| {
        m.inject(0, account(ID, AccountKind::HumanInteractive, LockState::Unlocked));
    });
    advance(cx, 100);
    assert!(!exists(cx, handle, "account-recover-acct-r"), "unlocked");

    // An AI identity has no phrase of its own.
    let (handle, shell) = world(cx);
    mock(cx, &shell, |m| {
        m.inject(0, account("acct-ai", AccountKind::HeadlessAi, LockState::Locked));
    });
    advance(cx, 100);
    assert!(exists(cx, handle, "account-recover-acct-r"));
    assert!(!exists(cx, handle, "account-recover-acct-ai"), "not for an AI identity");
}

#[gpui_kit::test]
fn the_entry_is_not_there_before_the_daemon_offers_the_phrase(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s13_born_v3");
    mock(cx, &shell, |m| {
        m.inject(0, account(ID, AccountKind::HumanInteractive, LockState::Locked));
    });
    advance(cx, 500);
    assert!(!with_store(cx, &shell, |s| s.offers_born_v3_creation()));
    assert!(exists(cx, handle, "unlock"), "the account is listed");
    assert!(!exists(cx, handle, "account-recover-acct-r"), "an older daemon: no entry");
}

#[gpui_kit::test]
fn nothing_is_sent_until_the_user_continues_and_nothing_secret_until_the_fingerprint(
    cx: &mut TestAppContext,
) {
    let (handle, shell) = world(cx);
    // Nothing started it: no recovery request, no record.
    assert_eq!(count(cx, &shell, "account.recover"), 0);
    assert!(record(cx, &shell).is_none());
    open_recovery(cx, handle);
    assert!(exists(cx, handle, "recovery-intro"));
    assert!(!exists(cx, handle, "account-recover-acct-r"), "it takes the forms' place");
    advance(cx, 300);
    assert_eq!(count(cx, &shell, "account.recover"), 0, "opening it sends nothing");
    // The warning and what is needed are announced before anything else.
    assert!(has_label(cx, handle, &tr("recovery.intro.warning")));
    assert!(has_label(cx, handle, &tr_args("recovery.intro.title", &[("name", "Alex")])));
    // Continue asks for the fingerprint only: one request, no secret.
    click(cx, handle, "recovery-continue");
    assert_eq!(pending(cx, &shell), Some(RecoveryPending::Begin));
    assert_eq!(disabled(cx, handle, "recovery-continue"), Some(true), "a second press waits");
    click(cx, handle, "recovery-continue");
    advance(cx, 300);
    assert_eq!(count(cx, &shell, "account.recover.begin"), 1);
    assert_eq!(last_body(cx, &shell, "account.recover.begin")["account_id"], ID);
    assert_eq!(phase(cx, &shell), Some(RecoveryPhase::Prepared));
    assert_eq!(count(cx, &shell, "account.recover.confirm"), 0);
    assert!(exists(cx, handle, "recovery-check"));
}

#[gpui_kit::test]
fn the_fingerprint_is_shown_and_must_be_typed_before_the_phrase_can_be_sent(
    cx: &mut TestAppContext,
) {
    let (handle, shell) = world(cx);
    to_check(cx, handle);
    let shown = tr_args("recovery.fingerprint.shown", &[("value", &grouped())]).to_string();
    assert!(has_label(cx, handle, &shown), "{shown}");
    assert!(has_label(cx, handle, &tr("recovery.fingerprint.help")));
    assert_eq!(disabled(cx, handle, "recovery-submit"), Some(true), "nothing typed");

    // The words and the passphrase alone are not enough.
    type_into(cx, handle, "recovery-phrase-input", &demo_phrase());
    type_into(cx, handle, "recovery-passphrase-input", PASSPHRASE);
    assert_eq!(disabled(cx, handle, "recovery-submit"), Some(true), "the fingerprint is missing");
    // A fingerprint that is not this account's is named.
    type_into(
        cx,
        handle,
        "recovery-fingerprint-input",
        "k3J9xQ2mP8vL1nRtYb6WcZ4dHf7GsA0eUiO5TqMaV-X",
    );
    assert!(has_label(cx, handle, &tr("recovery.fingerprint.mismatch")));
    assert_eq!(disabled(cx, handle, "recovery-submit"), Some(true));
    let role = in_window(cx, handle, |window, _| window.find("recovery-fingerprint-state").role());
    assert_eq!(role, Some(Role::Alert));
    // The right one, with or without the groups' spaces, is confirmed.
    let panel = panel(cx, &shell);
    in_window(cx, handle, |window, cx| {
        let input = panel.read(cx).inputs()[0].clone();
        input.update(cx, |input, cx| input.set_value("", window, cx));
    });
    type_into(cx, handle, "recovery-fingerprint-input", &grouped());
    assert!(has_label(cx, handle, &tr("recovery.fingerprint.match")));
    assert_ne!(disabled(cx, handle, "recovery-submit"), Some(true));
    assert_eq!(count(cx, &shell, "account.recover.confirm"), 0, "nothing was sent meanwhile");
}

#[gpui_kit::test]
fn the_phrase_is_counted_and_a_word_that_cannot_be_one_is_named(cx: &mut TestAppContext) {
    let (handle, shell) = world(cx);
    to_check(cx, handle);
    type_into(cx, handle, "recovery-fingerprint-input", DEMO_ANCHOR);
    type_into(cx, handle, "recovery-passphrase-input", PASSPHRASE);
    let ten = (1..=10).map(demo_word).collect::<Vec<_>>().join(" ");
    type_into(cx, handle, "recovery-phrase-input", &ten);
    let count = tr_args("recovery.phrase.count", &[("count", "10")]).to_string();
    assert!(has_label(cx, handle, &count), "{count}");
    assert_eq!(disabled(cx, handle, "recovery-submit"), Some(true), "10 of 24");
    let panel = panel(cx, &shell);
    in_window(cx, handle, |window, cx| {
        let input = panel.read(cx).inputs()[1].clone();
        input.update(cx, |input, cx| input.set_value(format!("{ten} xqzvw "), window, cx));
    });
    let unknown = tr_args("recovery.phrase.unknown", &[("number", "11")]).to_string();
    assert!(has_label(cx, handle, &unknown), "{unknown}");
    let role = in_window(cx, handle, |window, _| window.find("recovery-phrase-state").role());
    assert_eq!(role, Some(Role::Alert));
}

#[gpui_kit::test]
fn sending_clears_every_input_at_once_and_the_words_travel_beside_the_body(
    cx: &mut TestAppContext,
) {
    let (handle, shell) = world(cx);
    to_check(cx, handle);
    type_all(cx, handle, &grouped(), &demo_phrase(), PASSPHRASE);
    assert!(has_label(cx, handle, &tr("recovery.phrase.ready")));
    click(cx, handle, "recovery-submit");
    // Before the daemon answers: nothing typed is left, and the wait is neutral.
    assert!(inputs_empty(cx, &shell), "the inputs were cleared at once");
    assert_eq!(pending(cx, &shell), Some(RecoveryPending::Confirm));
    assert!(exists(cx, handle, "recovery-checking"));
    let role = in_window(cx, handle, |window, _| window.find("recovery-checking-text").role());
    assert_eq!(role, Some(Role::Status));
    advance(cx, 300);
    assert_eq!(count(cx, &shell, "account.recover.confirm"), 1);
    let body = last_body(cx, &shell, "account.recover.confirm");
    assert_eq!(body["confirmed_anchor"], DEMO_ANCHOR);
    let text = body.to_string();
    assert!(!text.contains(PASSPHRASE) && !text.contains(demo_word(3)), "{text}");

    // The result says what is true, and the account is open.
    assert!(matches!(phase(cx, &shell), Some(RecoveryPhase::Recovered(_))));
    assert!(exists(cx, handle, "recovery-done"));
    assert!(has_label(cx, handle, &tr_args("recovery.done.text", &[("name", "Alex")])));
    assert!(has_label(cx, handle, &tr("ceremony.done.state_text")));
    assert_eq!(
        with_store(cx, &shell, |s| s.account(&AccountId::new(ID)).map(|a| a.lock_state)),
        Some(LockState::Unlocked)
    );
    // The result stays in view though the account is unlocked by now.
    assert_eq!(page(cx, &shell), Page::Entry(EntryStep::Account));
    click(cx, handle, "recovery-done-close");
    advance(cx, 200);
    assert_eq!(page(cx, &shell), Page::Workspace);
}

#[gpui_kit::test]
fn wrong_words_a_wrong_passphrase_and_another_fingerprint_change_nothing(cx: &mut TestAppContext) {
    // Words that are list words but not this account's.
    let wrong = (1..=24).map(|n| demo_word(n + 1)).collect::<Vec<_>>().join(" ");
    let (handle, shell) = world(cx);
    to_check(cx, handle);
    type_all(cx, handle, DEMO_ANCHOR, &wrong, PASSPHRASE);
    click(cx, handle, "recovery-submit");
    advance(cx, 300);
    assert_eq!(phase(cx, &shell), Some(RecoveryPhase::Failed(CeremonyFailure::ConfirmationFailed)));
    assert!(has_label(cx, handle, &tr("recovery.ended.mismatch")), "the refusal is announced");
    let role = in_window(cx, handle, |window, _| window.find("recovery-ended-text").role());
    assert_eq!(role, Some(Role::Alert));
    assert!(inputs_empty(cx, &shell));
    assert_eq!(
        with_store(cx, &shell, |s| s.account(&AccountId::new(ID)).map(|a| a.lock_state)),
        Some(LockState::Locked),
        "nothing changed"
    );

    // A wrong passphrase of the account.
    click(cx, handle, "recovery-restart");
    assert!(exists(cx, handle, "recovery-intro"), "starting over begins with the explanation");
    click(cx, handle, "recovery-continue");
    advance(cx, 300);
    type_all(cx, handle, DEMO_ANCHOR, &demo_phrase(), "not the passphrase");
    click(cx, handle, "recovery-submit");
    advance(cx, 300);
    assert_eq!(phase(cx, &shell), Some(RecoveryPhase::Failed(CeremonyFailure::Invalid)));
    assert!(has_label(cx, handle, &tr("recovery.ended.invalid")));
    click(cx, handle, "recovery-close");
    assert!(exists(cx, handle, "account-recover-acct-r"), "back to the account list");
}

#[gpui_kit::test]
fn cancel_is_on_every_screen_escape_cancels_and_nothing_typed_stays(cx: &mut TestAppContext) {
    let (handle, shell) = world(cx);
    // The first screen: Back leaves, nothing was started.
    open_recovery(cx, handle);
    assert!(exists(cx, handle, "recovery-cancel"));
    click(cx, handle, "recovery-cancel");
    assert!(exists(cx, handle, "account-recover-acct-r"));
    assert_eq!(count(cx, &shell, "account.recover"), 0);

    // The fingerprint screen: cancel erases the daemon's copy and what is typed.
    to_check(cx, handle);
    assert!(exists(cx, handle, "recovery-cancel"));
    type_all(cx, handle, "k3J9", "abandon ability", "secret");
    click(cx, handle, "recovery-cancel");
    advance(cx, 300);
    assert_eq!(phase(cx, &shell), Some(RecoveryPhase::Cancelled));
    assert!(inputs_empty(cx, &shell));
    assert!(has_label(cx, handle, &tr("recovery.ended.cancelled")));
    click(cx, handle, "recovery-close");

    // Escape does the same.
    to_check(cx, handle);
    type_all(cx, handle, "k3J9", "abandon ability", "secret");
    in_window(cx, handle, |window, cx| window.press("escape", cx));
    advance(cx, 300);
    assert_eq!(phase(cx, &shell), Some(RecoveryPhase::Cancelled));
    assert!(inputs_empty(cx, &shell));
    assert_eq!(count(cx, &shell, "account.recover.cancel"), 2);
}

#[gpui_kit::test]
fn the_five_minutes_are_visible_and_an_expired_recovery_keeps_nothing(cx: &mut TestAppContext) {
    let (handle, shell) = world(cx);
    to_check(cx, handle);
    let first = in_window(cx, handle, |window, _| {
        window.find("recovery-countdown").label().map(str::to_owned)
    })
    .expect("countdown");
    assert!(first.contains("5:00") || first.contains("4:5"), "{first}");
    type_all(cx, handle, "k3J9", "abandon ability", "secret");
    jump(cx, &shell, u64::try_from(CEREMONY_TTL_MS).unwrap_or(0));
    assert!(exists(cx, handle, "recovery-ended"));
    assert!(has_label(cx, handle, &tr("recovery.ended.expired")), "the expiry is announced");
    assert!(inputs_empty(cx, &shell), "nothing typed stays");
    assert!(effects(cx, &shell).iter().any(|l| l == "account.recover.cancel"));
    let panel = panel(cx, &shell);
    assert!(cx.update(|cx| panel.read(cx).expired_seen()));
    advance(cx, 1_000);
    assert!(has_label(cx, handle, &tr("recovery.ended.expired")), "and it stays");
}

#[gpui_kit::test]
fn a_recovery_record_nobody_asked_for_is_never_shown(cx: &mut TestAppContext) {
    let (handle, shell) = world(cx);
    // The daemon (or the network through it) reports a recovery the user
    // never started.
    let unasked = RecoveryCeremony {
        phase: RecoveryPhase::Prepared,
        account_id: AccountId::new(ID),
        anchor: DEMO_ANCHOR.to_owned(),
        ceremony_id: "recovery-elsewhere".to_owned(),
        expires_at: Timestamp(i64::MAX / 2),
    };
    mock(cx, &shell, |m| m.inject(0, DomainEvent::RecoveryUpdated(unasked)));
    advance(cx, 300);
    assert!(record(cx, &shell).is_some_and(|r| r.is_active()), "the store holds it");
    assert!(!exists(cx, handle, "recovery"), "no screen opened by itself");
    assert!(exists(cx, handle, "account-recover-acct-r"), "the account list stays");
    // Opening the panel is the user's: it starts at the explanation, not at
    // someone else's fingerprint, and sends nothing.
    open_recovery(cx, handle);
    assert!(exists(cx, handle, "recovery-intro"));
    assert!(!exists(cx, handle, "recovery-check"));
    assert_eq!(count(cx, &shell, "account.recover"), 0);
}

#[gpui_kit::test]
fn closing_the_window_tells_the_daemon_to_let_go_and_clears_what_is_typed(cx: &mut TestAppContext) {
    let (handle, shell) = world(cx);
    to_check(cx, handle);
    type_all(cx, handle, "k3J9", "abandon ability", "secret");
    assert!(gpui_kit::VisualTestContext::from_window(handle.into(), cx).simulate_close());
    cx.run_until_parked();
    assert_eq!(count(cx, &shell, "account.recover.cancel"), 1);
    assert!(inputs_empty(cx, &shell));
}

#[gpui_kit::test]
fn focus_lands_on_an_input_or_the_text_never_on_a_button(cx: &mut TestAppContext) {
    let (handle, shell) = world(cx);
    open_recovery(cx, handle);
    advance(cx, 100);
    assert_eq!(target(cx, handle, &shell), Some("recovery-heading"), "the intro: its heading");
    let (role, name) = in_window(cx, handle, |window, _| {
        let heading = window.find("recovery-heading");
        (heading.role(), heading.label().map(str::to_owned))
    });
    assert_eq!(role, Some(Role::Heading));
    assert_eq!(name, Some(tr_args("recovery.intro.title", &[("name", "Alex")]).to_string()));
    // A stray Enter or Space on the intro starts nothing.
    in_window(cx, handle, |window, cx| window.press("enter", cx));
    in_window(cx, handle, |window, cx| window.press("space", cx));
    advance(cx, 300);
    assert_eq!(count(cx, &shell, "account.recover"), 0);

    click(cx, handle, "recovery-continue");
    advance(cx, 300);
    assert_eq!(target(cx, handle, &shell), Some("recovery-fingerprint-input"));
    type_all(cx, handle, &grouped(), &demo_phrase(), PASSPHRASE);
    click(cx, handle, "recovery-submit");
    assert_eq!(target(cx, handle, &shell), Some("recovery-checking-text"));
    advance(cx, 300);
    assert_eq!(target(cx, handle, &shell), Some("recovery-heading"), "the result: its heading");
    click(cx, handle, "recovery-done-close");

    // A refusal: the error itself.
    mock(cx, &shell, |m| {
        m.inject(0, account(ID, AccountKind::HumanInteractive, LockState::Locked));
    });
    advance(cx, 200);
    to_check(cx, handle);
    click(cx, handle, "recovery-cancel");
    advance(cx, 300);
    assert_eq!(target(cx, handle, &shell), Some("recovery-ended-text"));
}

#[gpui_kit::test]
fn every_control_of_the_recovery_has_a_role_and_a_name(cx: &mut TestAppContext) {
    const INTERACTIVE: [Role; 4] = [Role::Button, Role::TextInput, Role::PasswordInput, Role::Link];
    let unnamed = |cx: &mut TestAppContext, handle: Handle| -> Vec<String> {
        in_window(cx, handle, |window, _| {
            INTERACTIVE
                .iter()
                .flat_map(|role| window.find_all_by_role(*role))
                .filter(|e| e.visible() && e.label().is_none_or(|l| l.trim().is_empty()))
                .map(|e| format!("{:?} {:?}", e.role(), e.path().last()))
                .collect()
        })
    };
    let (handle, _shell) = world(cx);
    assert_eq!(unnamed(cx, handle), Vec::<String>::new(), "the account list");
    open_recovery(cx, handle);
    assert_eq!(unnamed(cx, handle), Vec::<String>::new(), "the intro");
    click(cx, handle, "recovery-continue");
    advance(cx, 300);
    type_all(cx, handle, "k3J9", "abandon ability", "secret");
    assert_eq!(unnamed(cx, handle), Vec::<String>::new(), "the fingerprint screen");
}

#[gpui_kit::test]
fn the_screens_speak_chinese_when_the_interface_does(cx: &mut TestAppContext) {
    let _guard = locale_guard();
    let (handle, _shell) = world(cx);
    cx.update(|cx| ramflux_desktop_ui::i18n::select_locale("zh-CN", cx));
    advance(cx, 100);
    open_recovery(cx, handle);
    let intro = tr_args("recovery.intro.title", &[("name", "Alex")]).to_string();
    assert!(!intro.is_ascii(), "{intro}");
    assert!(has_label(cx, handle, &intro));
    assert!(has_label(cx, handle, &tr("recovery.intro.warning")));
    click(cx, handle, "recovery-continue");
    advance(cx, 300);
    let shown = tr_args("recovery.fingerprint.shown", &[("value", &grouped())]).to_string();
    assert!(!shown.is_ascii(), "{shown}");
    assert!(has_label(cx, handle, &shown));
    cx.update(|cx| ramflux_desktop_ui::i18n::select_locale("en", cx));
}
