// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The born-v3 ceremonies through the production window on the real SDK
//! source (DESK-V3-02): the screens drive the bridge, the bridge speaks the
//! product's bus contract to a fake daemon (`FakeCustody`) on connections of
//! its own, and what the person sees is read through the accessibility tree.
//!
//! - creation, then lock, then a restart of the daemon and of the window,
//!   then unlock with the passphrase set at confirm;
//! - recovery of a locked account on a fresh window.

mod common;
mod sdk_fake;

use std::sync::Arc;

use common::*;
use gpui_kit::{AppContext as _, Entity, TestAppContext, WindowHandle};
use ramflux_desktop_core::{
    AccountId, CeremonyPhase, IntentKind, LockState, Networking, RecoveryPhase,
};
use ramflux_desktop_test::fake_custody::{self, FakeCustody};
use ramflux_desktop_test::mock_ceremony::{DEMO_POSITIONS, demo_word};
use ramflux_desktop_ui::Shell;
use ramflux_desktop_ui::i18n::{tr, tr_args};
use ramflux_desktop_ui::route::{EntryStep, Page};
use sdk_fake::wait;

type Handle = WindowHandle<gpui_kit::base::Root>;

const PASSPHRASE: &str = "river stone lantern";

/// A fresh device whose daemon offers the phrase, over `custody`.
fn fresh_device(
    cx: &mut TestAppContext,
    name: &str,
    custody: &Arc<FakeCustody>,
) -> (ramflux_desktop_test::fake_bus::FakeBus, Handle, Entity<Shell>) {
    let (fake, state, path) = sdk_fake::start_with_custody(name, custody.clone());
    {
        let mut state = state.lock().expect("state");
        state.born_v3 = true;
        // The user confirmed a node, but the daemon has no default yet.
        state.no_default = true;
        // The account list is the custody state's once it has accounts.
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
    (fake, handle, shell)
}

/// Reads the daemon's own store, where the ceremonies are kept (the
/// rendered store is the opened account's once there is one).
fn daemon<R>(
    cx: &mut TestAppContext,
    shell: &Entity<Shell>,
    f: impl FnOnce(&ramflux_desktop_core::Store) -> R,
) -> R {
    cx.update(|cx| f(shell.read(cx).store().read(cx).entry_store()))
}

fn ceremony_phase(cx: &mut TestAppContext, shell: &Entity<Shell>) -> Option<CeremonyPhase> {
    daemon(cx, shell, |s| s.ceremony().map(|c| c.phase.clone()))
}

fn held_words(cx: &mut TestAppContext, shell: &Entity<Shell>) -> usize {
    let panel = cx.update(|cx| shell.read(cx).entry().read(cx).ceremony().clone());
    cx.update(|cx| panel.read(cx).held().words())
}

#[gpui_kit::test]
fn a_created_account_is_locked_restarted_and_unlocked_with_its_passphrase(cx: &mut TestAppContext) {
    let custody = Arc::new(FakeCustody::new());
    let (fake, handle, shell) = fresh_device(cx, "ui-sdk-create", &custody);

    // Creation through the screens, on the real source.
    click(cx, handle, "account-create-phrase");
    type_into(cx, handle, "ceremony-name-input", "Alex");
    click(cx, handle, "ceremony-continue");
    wait(cx, "prepared", |cx| ceremony_phase(cx, &shell) == Some(CeremonyPhase::Prepared));
    click(cx, handle, "ceremony-show");
    wait(cx, "the words", |cx| ceremony_phase(cx, &shell) == Some(CeremonyPhase::PhraseReady));
    wait(cx, "taken", |cx| held_words(cx, &shell) == 24);
    assert!(has_label(cx, handle, &tr("ceremony.show.warning")));
    // The first of the words reads as the daemon wrote it.
    let first = tr_args("ceremony.show.cell", &[("number", "1"), ("word", demo_word(1))]);
    assert!(has_label(cx, handle, &first), "{first}");
    click(cx, handle, "ceremony-hide");
    assert_eq!(held_words(cx, &shell), 0, "erased before the daemon is asked");
    wait(cx, "asked", |cx| {
        matches!(ceremony_phase(cx, &shell), Some(CeremonyPhase::Challenged { .. }))
    });
    for (id, position) in
        ["ceremony-word-1", "ceremony-word-2", "ceremony-word-3"].into_iter().zip(DEMO_POSITIONS)
    {
        type_into(cx, handle, id, demo_word(usize::from(position)));
    }
    click(cx, handle, "ceremony-next");
    type_into(cx, handle, "ceremony-passphrase-input", PASSPHRASE);
    type_into(cx, handle, "ceremony-passphrase-repeat", PASSPHRASE);
    click(cx, handle, "ceremony-confirm-submit");
    wait(cx, "created", |cx| matches!(ceremony_phase(cx, &shell), Some(CeremonyPhase::Created(_))));
    let Some(CeremonyPhase::Created(made)) = ceremony_phase(cx, &shell) else { panic!("created") };
    let id = AccountId::new(&made.local_account_id);
    // The chosen node became the daemon's default before the ceremony
    // began, so the account connected inside confirm.
    assert_eq!(made.networking, Networking::Live);
    assert!(has_label(cx, handle, &tr("ceremony.done.live_state_text")));
    let seen = fake.seen();
    let order = |method: &str| seen.iter().position(|s| s.method == method);
    assert!(order("node.set_default").is_some(), "the node was made the default");
    assert!(order("node.set_default") < order("account.create"), "and before the begin");
    assert_eq!(custody.default_node().as_deref(), Some("n-harbor"));
    // The strip does not say the daemon is locked (L3: that was the mock scenario).
    assert_eq!(
        daemon(cx, &shell, |s| s.source_status()),
        ramflux_desktop_core::SourceStatus::Connected
    );
    // One connection for the whole ceremony, none shared with the rest.
    let connection = custody.connections_of("account.create");
    assert_eq!(connection.len(), 1);
    assert_eq!(custody.connections_of("account.create.confirm"), connection);
    // The words and the passphrase the daemon received are the typed ones.
    let (words, passphrase) = custody.last_confirm().expect("confirm");
    assert_eq!(words, DEMO_POSITIONS.map(|p| demo_word(usize::from(p)).to_owned()));
    assert_eq!(passphrase, PASSPHRASE);

    // Lock it from the app, as a person would.
    click(cx, handle, "ceremony-done-close");
    wait(cx, "the workspace", |cx| page(cx, &shell) == Page::Workspace);
    let root = cx.update(|cx| shell.read(cx).root().clone());
    in_window(cx, handle, |window, cx| {
        root.update(cx, |root, cx| {
            root.submit_secret(IntentKind::LockAccount { account_id: id.clone() }, window, cx)
        });
    });
    wait(cx, "locked", |cx| {
        daemon(cx, &shell, |s| s.account(&id).map(|a| a.lock_state)) == Some(LockState::Locked)
            && matches!(page(cx, &shell), Page::Entry(_))
    });

    // The daemon restarts and the app starts afresh.
    fake.shutdown();
    custody.restart();
    let (fake, handle, shell) = fresh_device(cx, "ui-sdk-create-2", &custody);
    wait(cx, "the account is listed locked", |cx| {
        daemon(cx, &shell, |s| s.account(&id).map(|a| a.lock_state)) == Some(LockState::Locked)
    });
    assert_eq!(
        daemon(cx, &shell, |s| s.account(&id).map(|a| a.networking)),
        Some(Networking::Live)
    );
    assert!(exists(cx, handle, "unlock"));
    assert!(!exists(cx, handle, "account-networking-acct-native-1"), "connected: no tag");

    // The wrong passphrase opens nothing; the one set at confirm does.
    type_into(cx, handle, "passphrase", "not the passphrase");
    click(cx, handle, "unlock");
    wait(cx, "refused", |cx| with_store(cx, &shell, |s| !s.failures().is_empty()));
    assert_eq!(
        daemon(cx, &shell, |s| s.account(&id).map(|a| a.lock_state)),
        Some(LockState::Locked)
    );
    let panel = cx.update(|cx| shell.read(cx).entry().read(cx).passphrase().clone());
    in_window(cx, handle, |window, cx| panel.update(cx, |p, cx| p.set_value("", window, cx)));
    type_into(cx, handle, "passphrase", PASSPHRASE);
    click(cx, handle, "unlock");
    wait(cx, "unlocked", |cx| page(cx, &shell) == Page::Workspace);
    assert_eq!(
        daemon(cx, &shell, |s| s.account(&id).map(|a| a.lock_state)),
        Some(LockState::Unlocked)
    );
    fake.shutdown();
}

#[gpui_kit::test]
fn a_locked_account_is_recovered_from_its_phrase_through_the_real_source(cx: &mut TestAppContext) {
    let custody = Arc::new(FakeCustody::new());
    custody.add_locked_account("acct-r", "Alex", PASSPHRASE);
    let (fake, handle, shell) = fresh_device(cx, "ui-sdk-recover", &custody);
    wait(cx, "the account is listed", |cx| {
        daemon(cx, &shell, |s| s.account(&AccountId::new("acct-r")).is_some())
    });
    assert!(custody.connections_of("account.recover.begin").is_empty(), "nothing started it");
    click(cx, handle, "account-recover-acct-r");
    click(cx, handle, "recovery-continue");
    wait(cx, "prepared", |cx| {
        daemon(cx, &shell, |s| s.recovery().map(|r| r.phase.clone()))
            == Some(RecoveryPhase::Prepared)
    });
    let anchor = custody.anchor_of("acct-r").expect("anchor");
    type_into(cx, handle, "recovery-fingerprint-input", &anchor);
    type_into(cx, handle, "recovery-phrase-input", &fake_custody::phrase());
    type_into(cx, handle, "recovery-passphrase-input", PASSPHRASE);
    click(cx, handle, "recovery-submit");
    wait(cx, "recovered", |cx| {
        matches!(
            daemon(cx, &shell, |s| s.recovery().map(|r| r.phase.clone())),
            Some(RecoveryPhase::Recovered(_))
        )
    });
    assert!(has_label(cx, handle, &tr_args("recovery.done.text", &[("name", "Alex")])));
    assert_eq!(
        custody.connections_of("account.recover.begin"),
        custody.connections_of("account.recover.confirm"),
        "one connection for the whole recovery"
    );
    click(cx, handle, "recovery-done-close");
    wait(cx, "the workspace", |cx| page(cx, &shell) == Page::Workspace);
    fake.shutdown();
}

#[gpui_kit::test]
fn the_banner_of_a_pending_account_clears_the_moment_the_daemon_connects_it(
    cx: &mut TestAppContext,
) {
    let custody = Arc::new(FakeCustody::new());
    // The node cannot be reached when the account is made.
    custody.set_live(false);
    let (fake, handle, shell) = fresh_device(cx, "ui-sdk-banner", &custody);
    click(cx, handle, "account-create-phrase");
    type_into(cx, handle, "ceremony-name-input", "Alex");
    click(cx, handle, "ceremony-continue");
    wait(cx, "prepared", |cx| ceremony_phase(cx, &shell) == Some(CeremonyPhase::Prepared));
    click(cx, handle, "ceremony-show");
    wait(cx, "taken", |cx| held_words(cx, &shell) == 24);
    click(cx, handle, "ceremony-hide");
    wait(cx, "asked", |cx| {
        matches!(ceremony_phase(cx, &shell), Some(CeremonyPhase::Challenged { .. }))
    });
    for (id, position) in
        ["ceremony-word-1", "ceremony-word-2", "ceremony-word-3"].into_iter().zip(DEMO_POSITIONS)
    {
        type_into(cx, handle, id, demo_word(usize::from(position)));
    }
    click(cx, handle, "ceremony-next");
    type_into(cx, handle, "ceremony-passphrase-input", PASSPHRASE);
    type_into(cx, handle, "ceremony-passphrase-repeat", PASSPHRASE);
    click(cx, handle, "ceremony-confirm-submit");
    wait(cx, "created", |cx| matches!(ceremony_phase(cx, &shell), Some(CeremonyPhase::Created(_))));
    // The done screen says it is connecting, to the node the user chose.
    let text = tr_args("ceremony.done.text", &[("name", "Alex"), ("node", "n-harbor")]);
    assert!(has_label(cx, handle, &text), "{text}");
    click(cx, handle, "ceremony-done-close");
    wait(cx, "the workspace", |cx| page(cx, &shell) == Page::Workspace);
    wait(cx, "the banner", |cx| exists(cx, handle, "networking-banner"));
    // The daemon connects the account later: no unlock, no restart.
    custody.go_live("acct-native-1");
    wait(cx, "the banner clears", |cx| {
        crate::common::advance(cx, 20);
        !exists(cx, handle, "networking-banner")
    });
    fake.shutdown();
}
