// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Account creation with a recovery phrase (born-v3), in the production
//! window on the mock source, read through the accessibility tree: the
//! entry is there only when the daemon says it offers the ceremony; the 24
//! words are shown once, numbered, and covered while the window is not
//! active; "I wrote them down" drops the phrase from the UI state before
//! the daemon is asked for the words; three words are typed back (with
//! prefix completion) for the one attempt; the five minutes are visible
//! when they pass; cancel is always there; and the result says the account
//! is networking pending and no more.

mod common;

use common::*;
use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{AppContext as _, Entity, Focusable as _, Role, TestAppContext, WindowHandle};
use ramflux_desktop_core::{
    CEREMONY_TTL_MS, CeremonyFailure, CeremonyPhase, Networking, PHRASE_WORDS,
};
use ramflux_desktop_test::mock_ceremony::{DEMO_POSITIONS, demo_word};
use ramflux_desktop_ui::Shell;
use ramflux_desktop_ui::entry::{CeremonyPanel, Pending};
use ramflux_desktop_ui::i18n::{tr, tr_args};
use ramflux_desktop_ui::route::{EntryStep, Page};
use ramflux_desktop_ui::settings::backup::text::Strength;

type Handle = WindowHandle<gpui_kit::base::Root>;

/// The window on the account step, before the daemon offers the ceremony.
fn account_step(cx: &mut TestAppContext) -> (Handle, Entity<Shell>) {
    let (handle, shell) = open(cx, "s13_born_v3");
    // A test window starts inactive; a person looks at an active one.
    activate(cx, handle);
    advance(cx, 500);
    assert_eq!(page(cx, &shell), Page::Entry(EntryStep::Account));
    (handle, shell)
}

/// The window with the entry offered.
fn offered(cx: &mut TestAppContext) -> (Handle, Entity<Shell>) {
    let (handle, shell) = account_step(cx);
    advance(cx, 2_000);
    (handle, shell)
}

fn panel(cx: &mut TestAppContext, shell: &Entity<Shell>) -> Entity<CeremonyPanel> {
    cx.update(|cx| shell.read(cx).entry().read(cx).ceremony().clone())
}

fn phase(cx: &mut TestAppContext, shell: &Entity<Shell>) -> Option<CeremonyPhase> {
    with_store(cx, shell, |s| s.ceremony().map(|c| c.phase.clone()))
}

/// Words the panel holds, as the state struct counts them.
fn held_words(cx: &mut TestAppContext, shell: &Entity<Shell>) -> usize {
    let panel = panel(cx, shell);
    cx.update(|cx| panel.read(cx).held().words())
}

fn label_of(cx: &mut TestAppContext, handle: Handle, id: &'static str) -> Option<String> {
    in_window(cx, handle, move |window, _| {
        window.try_find(id).and_then(|e| e.label().map(str::to_owned))
    })
}

fn cell(number: usize) -> String {
    tr_args("ceremony.show.cell", &[("number", &number.to_string()), ("word", demo_word(number))])
        .to_string()
}

/// Whether any painted node is named with one of the phrase's words.
fn any_word_named(cx: &mut TestAppContext, handle: Handle) -> bool {
    (1..=PHRASE_WORDS).any(|n| has_label(cx, handle, &cell(n)))
}

/// Through the name step to the ready screen.
fn begin(cx: &mut TestAppContext, handle: Handle) {
    if exists(cx, handle, "account-create-phrase") {
        click(cx, handle, "account-create-phrase");
    }
    type_into(cx, handle, "ceremony-name-input", "Alex");
    click(cx, handle, "ceremony-continue");
    advance(cx, 200);
}

/// Through the ready screen to the 24 words.
fn show(cx: &mut TestAppContext, handle: Handle) {
    click(cx, handle, "ceremony-show");
    advance(cx, 200);
}

/// To the confirmation screen.
fn challenged(cx: &mut TestAppContext, handle: Handle) {
    begin(cx, handle);
    show(cx, handle);
    click(cx, handle, "ceremony-hide");
    advance(cx, 200);
}

fn type_words(cx: &mut TestAppContext, handle: Handle, words: [&str; 3]) {
    for (id, word) in
        ["ceremony-word-1", "ceremony-word-2", "ceremony-word-3"].into_iter().zip(words)
    {
        type_into(cx, handle, id, word);
    }
}

/// Types the passphrase twice on the passphrase step.
fn type_passphrase(
    cx: &mut TestAppContext,
    handle: Handle,
    shell: &Entity<Shell>,
    first: &str,
    second: &str,
) {
    // Typing appends: start from empty inputs.
    let panel = panel(cx, shell);
    in_window(cx, handle, |window, cx| {
        let inputs: Vec<_> = panel.read(cx).passphrase_inputs().into_iter().cloned().collect();
        for input in inputs {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
    });
    type_into(cx, handle, "ceremony-passphrase-input", first);
    type_into(cx, handle, "ceremony-passphrase-repeat", second);
}

const PASSPHRASE: &str = "river stone lantern";

/// The three right words, then the passphrase step.
fn to_passphrase(cx: &mut TestAppContext, handle: Handle) {
    type_words(cx, handle, right_words());
    click(cx, handle, "ceremony-next");
}

fn right_words() -> [&'static str; 3] {
    DEMO_POSITIONS.map(|p| demo_word(usize::from(p)))
}

fn deactivate(cx: &mut TestAppContext, handle: Handle) {
    gpui_kit::VisualTestContext::from_window(handle.into(), cx).deactivate_window();
    cx.run_until_parked();
}

fn activate(cx: &mut TestAppContext, handle: Handle) {
    cx.update_window(handle.into(), |_, window, _| window.activate_window()).expect("window");
    cx.run_until_parked();
}

fn disabled(cx: &mut TestAppContext, handle: Handle, id: &'static str) -> Option<bool> {
    in_window(cx, handle, move |window, _| window.find(id).disabled())
}

#[gpui_kit::test]
fn the_entry_is_there_only_when_the_daemon_offers_the_ceremony(cx: &mut TestAppContext) {
    let (handle, shell) = account_step(cx);
    assert!(exists(cx, handle, "account-create"), "an older daemon keeps the passphrase form");
    let ai = ramflux_desktop_ui::i18n::account_kind(ramflux_desktop_core::AccountKind::HeadlessAi);
    assert!(has_label(cx, handle, &ai), "the kind is chosen on such a daemon");
    assert!(!exists(cx, handle, "account-create-phrase"), "not offered yet: no entry");
    assert!(!has_label(cx, handle, &tr("ceremony.entry.button")));
    assert!(!with_store(cx, &shell, |s| s.offers_born_v3_creation()));

    advance(cx, 2_000);
    assert!(with_store(cx, &shell, |s| s.offers_born_v3_creation()));
    // The phrase is the creation path of such a daemon, not an extra: the
    // passphrase form is gone.
    assert!(!exists(cx, handle, "account-create"), "no one-shot creation beside it");
    assert!(!exists(cx, handle, "account-secret"));
    assert!(exists(cx, handle, "account-create-phrase"), "the phrase creation is the path");
    assert!(!has_label(cx, handle, &ai), "a person's account only: no kind to choose");
    assert!(has_label(cx, handle, &tr("ceremony.entry.button")));
}

#[gpui_kit::test]
fn the_first_screens_ask_for_a_name_and_warn_before_any_word_is_shown(cx: &mut TestAppContext) {
    let (handle, shell) = offered(cx);
    click(cx, handle, "account-create-phrase");
    assert!(exists(cx, handle, "ceremony-name"));
    assert!(!exists(cx, handle, "account-create-phrase"), "the ceremony takes the forms' place");
    assert_eq!(disabled(cx, handle, "ceremony-continue"), Some(true), "a name is needed");

    type_into(cx, handle, "ceremony-name-input", "Alex");
    click(cx, handle, "ceremony-continue");
    advance(cx, 200);
    assert_eq!(phase(cx, &shell), Some(CeremonyPhase::Prepared));
    assert!(exists(cx, handle, "ceremony-ready"));
    assert!(has_label(cx, handle, &tr("ceremony.ready.warning")), "the warning is announced");
    assert!(!exists(cx, handle, "ceremony-grid"), "nothing is shown before the user asks");
    assert_eq!(held_words(cx, &shell), 0);
    assert!(exists(cx, handle, "ceremony-cancel"), "cancel is there");
}

#[gpui_kit::test]
fn the_24_words_are_shown_numbered_with_the_warning_and_no_copy_button(cx: &mut TestAppContext) {
    let (handle, shell) = offered(cx);
    begin(cx, handle);
    show(cx, handle);
    assert_eq!(phase(cx, &shell), Some(CeremonyPhase::PhraseReady));
    assert_eq!(held_words(cx, &shell), PHRASE_WORDS);

    assert!(has_label(cx, handle, &tr("ceremony.show.warning")), "the warning is announced");
    for number in 1..=PHRASE_WORDS {
        assert!(
            has_label(cx, handle, &cell(number)),
            "word {number} is numbered: {}",
            cell(number)
        );
    }
    let items = in_window(cx, handle, |window, _| window.find_all_by_role(Role::ListItem).len());
    assert!(items >= PHRASE_WORDS, "the grid lists the words: {items}");
    // Nothing offers to copy them.
    let copy = in_window(cx, handle, |window, _| {
        window
            .find_all_by_role(Role::Button)
            .iter()
            .filter_map(|b| b.label().map(str::to_lowercase))
            .any(|label| label.contains("copy") || label.contains("clipboard"))
    });
    assert!(!copy, "no copy button");
    assert!(has_label(cx, handle, &tr("ceremony.show.no_copy")), "the missing copy button is said");
    assert!(exists(cx, handle, "ceremony-cancel"), "cancel is there");
    assert!(exists(cx, handle, "ceremony-countdown"), "the time left is shown");
}

#[gpui_kit::test]
fn the_words_are_covered_while_the_window_is_not_active(cx: &mut TestAppContext) {
    let (handle, shell) = offered(cx);
    begin(cx, handle);
    show(cx, handle);
    assert!(has_label(cx, handle, &cell(1)));

    deactivate(cx, handle);
    let panel = panel(cx, &shell);
    assert!(cx.update(|cx| panel.read(cx).held().is_covered()), "covered on blur");
    assert!(!any_word_named(cx, handle), "no word is drawn while covered");
    let hidden = tr_args("ceremony.show.cell_hidden", &[("number", "1")]).to_string();
    assert!(has_label(cx, handle, &hidden), "the cells say they are hidden");
    assert!(has_label(cx, handle, &tr("ceremony.show.covered")), "and why");
    assert_eq!(held_words(cx, &shell), PHRASE_WORDS, "covered, not lost: the phrase is shown once");

    // Not revealed while the window is still not active.
    click(cx, handle, "ceremony-reveal");
    assert!(!any_word_named(cx, handle));

    activate(cx, handle);
    assert!(!any_word_named(cx, handle), "refocusing does not reveal by itself");
    click(cx, handle, "ceremony-reveal");
    assert!(has_label(cx, handle, &cell(1)), "revealed on request");
}

#[gpui_kit::test]
fn hide_drops_the_phrase_from_the_ui_state_before_the_words_are_asked(cx: &mut TestAppContext) {
    let (handle, shell) = offered(cx);
    begin(cx, handle);
    show(cx, handle);
    assert_eq!(held_words(cx, &shell), PHRASE_WORDS);

    click(cx, handle, "ceremony-hide");
    // The panel's state struct holds no words: checked before the daemon
    // answered, and after.
    assert_eq!(held_words(cx, &shell), 0, "erased at once");
    advance(cx, 200);
    let panel = panel(cx, &shell);
    cx.update(|cx| {
        let held = panel.read(cx).held();
        assert!(!held.holds_phrase());
        assert_eq!(held.words(), 0);
        assert!(!format!("{held:?}").contains(demo_word(3)), "nothing to print");
    });
    assert_eq!(phase(cx, &shell), Some(CeremonyPhase::Challenged { positions: DEMO_POSITIONS }));
    assert!(!exists(cx, handle, "ceremony-grid"), "the grid is gone");
    assert!(!any_word_named(cx, handle), "no painted node carries a word");
    // The effect that asked for the words came after the erasure, and no
    // body of any effect carries a word.
    let labels = effects(cx, &shell);
    assert!(labels.iter().any(|l| l == "account.create.hidden_challenge"), "{labels:?}");
    for (label, body) in bodies(cx, &shell) {
        let text = body.to_string();
        assert!(
            (1..=PHRASE_WORDS).all(|n| !text.contains(&format!("\"{}\"", demo_word(n)))),
            "{label}"
        );
    }
}

#[gpui_kit::test]
fn the_asked_words_complete_from_a_prefix_and_confirming_creates_the_account(
    cx: &mut TestAppContext,
) {
    let (handle, shell) = offered(cx);
    challenged(cx, handle);
    assert!(exists(cx, handle, "ceremony-confirm"));
    for position in DEMO_POSITIONS {
        let label = tr_args("ceremony.confirm.word", &[("number", &position.to_string())]);
        assert!(has_label(cx, handle, &label), "word {position} is asked");
    }
    assert!(has_label(cx, handle, &tr("ceremony.confirm.once")), "the one attempt is said");
    assert_eq!(disabled(cx, handle, "ceremony-next"), Some(true));

    let [a, b, c] = right_words();
    // The first word as its first four letters.
    let prefix: String = a.chars().take(4).collect();
    type_words(cx, handle, [&prefix, b, c]);
    let reads = tr_args("ceremony.confirm.reads", &[("word", a)]).to_string();
    assert!(has_label(cx, handle, &reads), "the prefix is completed: {reads}");
    assert_ne!(disabled(cx, handle, "ceremony-next"), Some(true));

    // The words alone send nothing: the passphrase comes next, and only
    // then is the one confirm sent.
    click(cx, handle, "ceremony-next");
    assert!(exists(cx, handle, "ceremony-passphrase"));
    assert!(!exists(cx, handle, "ceremony-word-1"), "the words are no longer on screen");
    assert!(!effects(cx, &shell).iter().any(|l| l.starts_with("account.create.confirm")));
    type_passphrase(cx, handle, &shell, PASSPHRASE, PASSPHRASE);
    click(cx, handle, "ceremony-confirm-submit");
    advance(cx, 200);
    assert_eq!(
        effects(cx, &shell).iter().filter(|l| l.starts_with("account.create.confirm")).count(),
        1,
        "one confirm: the words and the passphrase together"
    );
    assert!(matches!(phase(cx, &shell), Some(CeremonyPhase::Created(_))));
    // The typed words are gone from the inputs.
    let panel = panel(cx, &shell);
    cx.update(|cx| {
        let panel = panel.read(cx);
        assert!(panel.word_inputs().iter().all(|i| i.read(cx).value().is_empty()));
        assert!(panel.passphrase_inputs().iter().all(|i| i.read(cx).value().is_empty()));
        assert!(!panel.holds_typed_words());
    });
    // The result says what is true and no more.
    assert!(exists(cx, handle, "ceremony-done"));
    assert!(has_label(cx, handle, &tr("ceremony.done.state_text")));
    let text = tr_args("ceremony.done.text", &[("name", "Alex")]).to_string();
    assert!(has_label(cx, handle, &text), "{text}");
    // The account is listed and unlocked by now, yet the result stays in
    // view until it is closed.
    assert_eq!(with_store(cx, &shell, |s| s.accounts().count()), 1);
    assert_eq!(page(cx, &shell), Page::Entry(EntryStep::Account));

    click(cx, handle, "ceremony-done-close");
    advance(cx, 100);
    // Now the account's own workspace opens, and it says what the account
    // cannot do yet.
    assert_eq!(page(cx, &shell), Page::Workspace);
    assert!(has_label(cx, handle, &tr("networking.pending_text")), "the account carries its state");
}

#[gpui_kit::test]
fn a_word_that_is_not_a_recovery_word_does_not_spend_the_attempt(cx: &mut TestAppContext) {
    let (handle, shell) = offered(cx);
    challenged(cx, handle);
    let [_, b, c] = right_words();
    type_words(cx, handle, ["zzzzz", b, c]);
    assert!(has_label(cx, handle, &tr("ceremony.confirm.unknown")));
    assert_eq!(disabled(cx, handle, "ceremony-next"), Some(true));
    click(cx, handle, "ceremony-next");
    advance(cx, 200);
    assert_eq!(
        phase(cx, &shell),
        Some(CeremonyPhase::Challenged { positions: DEMO_POSITIONS }),
        "nothing was sent"
    );
    assert!(!effects(cx, &shell).iter().any(|l| l.starts_with("account.create.confirm")));
}

#[gpui_kit::test]
fn a_wrong_word_ends_the_setup_with_the_backends_error(cx: &mut TestAppContext) {
    let (handle, shell) = offered(cx);
    challenged(cx, handle);
    let [a, b, _] = right_words();
    // A real recovery word, but not the one asked for.
    type_words(cx, handle, [a, b, demo_word(5)]);
    click(cx, handle, "ceremony-next");
    type_passphrase(cx, handle, &shell, PASSPHRASE, PASSPHRASE);
    click(cx, handle, "ceremony-confirm-submit");
    advance(cx, 200);
    assert_eq!(phase(cx, &shell), Some(CeremonyPhase::Failed(CeremonyFailure::ConfirmationFailed)));
    assert!(exists(cx, handle, "ceremony-ended"));
    assert!(has_label(cx, handle, &tr("ceremony.ended.mismatch")), "the error is announced");
    assert!(!exists(cx, handle, "ceremony-confirm"), "no second attempt");
    assert!(!exists(cx, handle, "ceremony-done"), "no account");
    assert_eq!(held_words(cx, &shell), 0);
    let panel = panel(cx, &shell);
    cx.update(|cx| {
        let panel = panel.read(cx);
        assert!(!panel.holds_typed_words());
        assert!(panel.passphrase_inputs().iter().all(|i| i.read(cx).value().is_empty()));
    });

    // Starting over begins with the name again and a fresh ceremony.
    click(cx, handle, "ceremony-restart");
    assert!(exists(cx, handle, "ceremony-name"));
    assert!(!exists(cx, handle, "ceremony-ended"));
}

#[gpui_kit::test]
fn the_five_minutes_are_visible_and_an_expired_setup_holds_nothing(cx: &mut TestAppContext) {
    let (handle, shell) = offered(cx);
    begin(cx, handle);
    show(cx, handle);
    let first = label_of(cx, handle, "ceremony-countdown").expect("countdown");
    assert!(first.contains("5:00") || first.contains("4:5"), "{first}");
    advance(cx, 61_000);
    let later = label_of(cx, handle, "ceremony-countdown").expect("countdown");
    assert_ne!(first, later, "the countdown moves");
    assert!(later.contains("3:5") || later.contains("3:4"), "{later}");
    assert_eq!(held_words(cx, &shell), PHRASE_WORDS);

    advance(cx, u64::try_from(CEREMONY_TTL_MS).unwrap_or(0));
    assert!(!exists(cx, handle, "ceremony-countdown"));
    assert!(exists(cx, handle, "ceremony-ended"));
    assert!(has_label(cx, handle, &tr("ceremony.ended.expired")), "the expiry is announced");
    let role = in_window(cx, handle, |window, _| window.find("ceremony-ended-text").role());
    assert_eq!(role, Some(Role::Alert));
    assert_eq!(held_words(cx, &shell), 0, "the phrase went with it");
    assert!(!any_word_named(cx, handle));
    let panel = panel(cx, &shell);
    assert!(cx.update(|cx| panel.read(cx).expired_seen()));
    // The daemon was told to let go, and the expiry stays in view after it did.
    assert!(effects(cx, &shell).iter().any(|l| l == "account.create.cancel"));
    advance(cx, 1_000);
    assert!(has_label(cx, handle, &tr("ceremony.ended.expired")));
    assert!(!has_label(cx, handle, &tr("ceremony.ended.cancelled")));
}

#[gpui_kit::test]
fn cancel_is_on_every_screen_and_erases_the_phrase(cx: &mut TestAppContext) {
    let (handle, shell) = offered(cx);
    begin(cx, handle);
    assert!(exists(cx, handle, "ceremony-cancel"), "ready screen");
    show(cx, handle);
    assert!(exists(cx, handle, "ceremony-cancel"), "phrase screen");
    click(cx, handle, "ceremony-cancel");
    advance(cx, 200);
    assert_eq!(phase(cx, &shell), Some(CeremonyPhase::Cancelled));
    assert_eq!(held_words(cx, &shell), 0, "cancelled: nothing held");
    assert!(!any_word_named(cx, handle));
    assert!(has_label(cx, handle, &tr("ceremony.ended.cancelled")));

    // From the confirmation too.
    click(cx, handle, "ceremony-restart");
    challenged(cx, handle);
    assert!(exists(cx, handle, "ceremony-cancel"), "confirm screen");
    type_words(cx, handle, [demo_word(3), "", ""]);
    click(cx, handle, "ceremony-cancel");
    advance(cx, 200);
    assert_eq!(phase(cx, &shell), Some(CeremonyPhase::Cancelled));
    let panel = panel(cx, &shell);
    cx.update(|cx| {
        assert!(panel.read(cx).word_inputs().iter().all(|i| i.read(cx).value().is_empty()));
    });
    // And from the passphrase step: the words and the passphrase go.
    click(cx, handle, "ceremony-restart");
    challenged(cx, handle);
    to_passphrase(cx, handle);
    type_passphrase(cx, handle, &shell, PASSPHRASE, "");
    assert!(exists(cx, handle, "ceremony-cancel"), "passphrase step");
    assert!(cx.update(|cx| panel.read(cx).holds_typed_words()));
    click(cx, handle, "ceremony-cancel");
    advance(cx, 200);
    assert_eq!(phase(cx, &shell), Some(CeremonyPhase::Cancelled));
    cx.update(|cx| {
        let panel = panel.read(cx);
        assert!(!panel.holds_typed_words());
        assert!(panel.passphrase_inputs().iter().all(|i| i.read(cx).value().is_empty()));
    });
    // Closing returns to the forms.
    click(cx, handle, "ceremony-close");
    assert!(exists(cx, handle, "account-create-phrase"));
}

#[gpui_kit::test]
fn backing_out_of_the_name_step_returns_to_the_forms(cx: &mut TestAppContext) {
    let (handle, _shell) = offered(cx);
    click(cx, handle, "account-create-phrase");
    assert!(exists(cx, handle, "ceremony-name"));
    click(cx, handle, "ceremony-cancel");
    assert!(exists(cx, handle, "account-create-phrase"));
    assert!(!exists(cx, handle, "ceremony-name"));
}

#[gpui_kit::test]
fn every_control_of_the_ceremony_has_a_role_and_a_name(cx: &mut TestAppContext) {
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
    let (handle, shell) = offered(cx);
    click(cx, handle, "account-create-phrase");
    assert_eq!(unnamed(cx, handle), Vec::<String>::new(), "name screen");
    type_into(cx, handle, "ceremony-name-input", "Alex");
    click(cx, handle, "ceremony-continue");
    advance(cx, 200);
    assert_eq!(unnamed(cx, handle), Vec::<String>::new(), "ready screen");
    show(cx, handle);
    assert_eq!(unnamed(cx, handle), Vec::<String>::new(), "phrase screen");
    click(cx, handle, "ceremony-hide");
    advance(cx, 200);
    type_into(cx, handle, "ceremony-word-1", "ab");
    assert_eq!(unnamed(cx, handle), Vec::<String>::new(), "confirm screen with suggestions");
    let word_panel = panel(cx, &shell);
    in_window(cx, handle, |window, cx| {
        let first = word_panel.read(cx).word_inputs()[0].clone();
        first.update(cx, |input, cx| input.set_value("", window, cx));
    });
    type_words(cx, handle, right_words());
    click(cx, handle, "ceremony-next");
    type_passphrase(cx, handle, &shell, "short", "other");
    assert_eq!(unnamed(cx, handle), Vec::<String>::new(), "passphrase step");
}

fn count(cx: &mut TestAppContext, shell: &Entity<Shell>, label: &str) -> usize {
    effects(cx, shell).iter().filter(|l| l.starts_with(label)).count()
}

#[gpui_kit::test]
fn a_double_press_sends_each_request_once(cx: &mut TestAppContext) {
    let (handle, shell) = offered(cx);
    click(cx, handle, "account-create-phrase");
    type_into(cx, handle, "ceremony-name-input", "Alex");
    // Two presses before the daemon answers.
    click(cx, handle, "ceremony-continue");
    assert_eq!(disabled(cx, handle, "ceremony-continue"), Some(true), "pending: off");
    assert_eq!(panel_pending(cx, &shell), Some(Pending::Begin));
    click(cx, handle, "ceremony-continue");
    advance(cx, 200);
    assert_eq!(count(cx, &shell, "account.create"), 1);
    assert_eq!(phase(cx, &shell), Some(CeremonyPhase::Prepared));
    assert_eq!(panel_pending(cx, &shell), None, "the answer ended the wait");

    click(cx, handle, "ceremony-show");
    assert_eq!(disabled(cx, handle, "ceremony-show"), Some(true));
    click(cx, handle, "ceremony-show");
    advance(cx, 200);
    assert_eq!(count(cx, &shell, "account.create.show_once"), 1);
    assert_eq!(phase(cx, &shell), Some(CeremonyPhase::PhraseReady), "the phrase was not lost");
    assert_eq!(held_words(cx, &shell), PHRASE_WORDS);

    // The same through the methods, where no button is left to be disabled.
    let panel = panel(cx, &shell);
    in_window(cx, handle, |window, cx| {
        panel.update(cx, |panel, cx| {
            panel.hide(window, cx);
            panel.hide(window, cx);
        });
    });
    advance(cx, 200);
    assert_eq!(count(cx, &shell, "account.create.hidden_challenge"), 1);
    type_words(cx, handle, right_words());
    click(cx, handle, "ceremony-next");
    type_passphrase(cx, handle, &shell, PASSPHRASE, PASSPHRASE);
    in_window(cx, handle, |window, cx| {
        panel.update(cx, |panel, cx| {
            panel.confirm(window, cx);
            panel.confirm(window, cx);
        });
    });
    advance(cx, 200);
    assert_eq!(count(cx, &shell, "account.create.confirm"), 1);
    assert!(matches!(phase(cx, &shell), Some(CeremonyPhase::Created(_))));
}

fn panel_pending(cx: &mut TestAppContext, shell: &Entity<Shell>) -> Option<Pending> {
    let panel = panel(cx, shell);
    cx.update(|cx| panel.read(cx).pending())
}

#[gpui_kit::test]
fn waiting_for_the_hide_answer_is_neutral_not_an_error(cx: &mut TestAppContext) {
    let (handle, shell) = offered(cx);
    begin(cx, handle);
    show(cx, handle);
    click(cx, handle, "ceremony-hide");
    // The phrase is gone and the daemon has not answered yet.
    assert_eq!(held_words(cx, &shell), 0);
    assert_eq!(phase(cx, &shell), Some(CeremonyPhase::PhraseReady));
    assert!(!exists(cx, handle, "ceremony-phrase-missing"), "no false error");
    assert!(!has_label(cx, handle, &tr("ceremony.show.missing")));
    assert!(exists(cx, handle, "ceremony-hiding"));
    let role = in_window(cx, handle, |window, _| window.find("ceremony-hiding-text").role());
    assert_eq!(role, Some(Role::Status), "announced politely, not as an alert");
    assert!(exists(cx, handle, "ceremony-cancel"), "cancel stays");
    advance(cx, 200);
    assert!(exists(cx, handle, "ceremony-confirm"));
    assert!(!exists(cx, handle, "ceremony-hiding"));
}

#[gpui_kit::test]
fn the_passphrase_follows_the_backup_passphrase_rules(cx: &mut TestAppContext) {
    let (handle, shell) = offered(cx);
    challenged(cx, handle);
    to_passphrase(cx, handle);
    assert!(exists(cx, handle, "ceremony-passphrase"));
    assert_eq!(disabled(cx, handle, "ceremony-confirm-submit"), Some(true), "nothing typed");

    type_passphrase(cx, handle, &shell, "short", "short");
    let hint = Strength::of("short").hint().to_string();
    assert!(has_label(cx, handle, &hint), "the strength is said: {hint}");
    assert_eq!(disabled(cx, handle, "ceremony-confirm-submit"), Some(true), "too short");

    type_passphrase(cx, handle, &shell, "river stone lantern", "river stone lanter");
    assert!(has_label(cx, handle, &tr("settings.backup.mismatch")), "a mismatch is named");
    assert_eq!(disabled(cx, handle, "ceremony-confirm-submit"), Some(true));
    // Nothing was sent meanwhile.
    assert_eq!(count(cx, &shell, "account.create.confirm"), 0);

    type_passphrase(cx, handle, &shell, "river stone lantern", "river stone lantern");
    assert!(!has_label(cx, handle, &tr("settings.backup.mismatch")));
    assert_ne!(disabled(cx, handle, "ceremony-confirm-submit"), Some(true));
}

#[gpui_kit::test]
fn the_account_carries_its_networking_state_from_the_list(cx: &mut TestAppContext) {
    let (handle, shell) = offered(cx);
    challenged(cx, handle);
    to_passphrase(cx, handle);
    type_passphrase(cx, handle, &shell, PASSPHRASE, PASSPHRASE);
    click(cx, handle, "ceremony-confirm-submit");
    advance(cx, 200);
    let account = with_store(cx, &shell, |s| s.accounts().next().cloned()).expect("listed");
    assert_eq!(account.networking, Networking::Pending);
    // The done screen shows it with a tag...
    assert!(has_label(cx, handle, &tr("ceremony.done.state_text")));
    click(cx, handle, "ceremony-done-close");
    advance(cx, 100);
    // ...and the account's own page keeps saying it.
    assert_eq!(page(cx, &shell), Page::Workspace);
    assert!(exists(cx, handle, "networking-banner"));
    assert!(has_label(cx, handle, &tr("networking.pending_text")));
}

#[gpui_kit::test]
fn focus_moves_to_the_first_control_of_each_step(cx: &mut TestAppContext) {
    let (handle, shell) = offered(cx);
    let panel = panel(cx, &shell);
    let focused = |cx: &mut TestAppContext, pick: fn(&CeremonyPanel) -> usize| {
        advance(cx, 100);
        in_window(cx, handle, |_, _| {});
        let panel = panel.clone();
        cx.update_window(handle.into(), move |_, window, cx| {
            let panel = panel.read(cx);
            let inputs = [
                panel.name_input().clone(),
                panel.word_inputs()[0].clone(),
                panel.passphrase_inputs()[0].clone(),
            ];
            let which = pick(panel);
            (
                inputs[which].read(cx).focus_handle(cx).is_focused(window),
                panel.has_focus(window, cx),
            )
        })
        .expect("window")
    };
    click(cx, handle, "account-create-phrase");
    assert!(focused(cx, |_| 0).0, "the name input");
    type_into(cx, handle, "ceremony-name-input", "Alex");
    click(cx, handle, "ceremony-continue");
    assert!(focused(cx, |_| 0).1, "the ready step: focus is in the panel, on its first control");
    show(cx, handle);
    assert!(focused(cx, |_| 0).1);
    click(cx, handle, "ceremony-hide");
    advance(cx, 200);
    assert!(focused(cx, |_| 1).0, "the first asked word");
    type_words(cx, handle, right_words());
    click(cx, handle, "ceremony-next");
    assert!(focused(cx, |_| 2).0, "the passphrase");
}

#[gpui_kit::test]
fn escape_asks_before_it_cancels(cx: &mut TestAppContext) {
    let (handle, shell) = offered(cx);
    begin(cx, handle);
    show(cx, handle);
    advance(cx, 100);
    in_window(cx, handle, |window, cx| window.press("escape", cx));
    assert!(exists(cx, handle, "ceremony-cancel-question"), "it asks");
    assert_eq!(phase(cx, &shell), Some(CeremonyPhase::PhraseReady), "nothing cancelled yet");
    assert_eq!(held_words(cx, &shell), PHRASE_WORDS);
    // Escape again, or "Keep going", goes on.
    click(cx, handle, "ceremony-cancel-no");
    assert!(!exists(cx, handle, "ceremony-cancel-question"));
    in_window(cx, handle, |window, cx| window.press("escape", cx));
    assert!(exists(cx, handle, "ceremony-cancel-question"));
    click(cx, handle, "ceremony-cancel-yes");
    advance(cx, 200);
    assert_eq!(phase(cx, &shell), Some(CeremonyPhase::Cancelled));
    assert_eq!(held_words(cx, &shell), 0);
}

#[gpui_kit::test]
fn closing_the_window_tells_the_daemon_to_let_go(cx: &mut TestAppContext) {
    let (handle, shell) = offered(cx);
    begin(cx, handle);
    show(cx, handle);
    assert_eq!(count(cx, &shell, "account.create.cancel"), 0);
    assert!(gpui_kit::VisualTestContext::from_window(handle.into(), cx).simulate_close());
    cx.run_until_parked();
    assert_eq!(count(cx, &shell, "account.create.cancel"), 1);
    assert_eq!(held_words(cx, &shell), 0);
}

#[gpui_kit::test]
fn the_word_cells_keep_their_width_in_a_narrow_window(cx: &mut TestAppContext) {
    let (handle, shell) = open_width(cx, "s13_born_v3", 520.);
    activate(cx, handle);
    advance(cx, 2_500);
    begin(cx, handle);
    show(cx, handle);
    assert_eq!(held_words(cx, &shell), PHRASE_WORDS);
    let (grid, cells) = in_window(cx, handle, |window, _| {
        let grid = window.find("ceremony-grid").bounds();
        let cells: Vec<_> = (1..=PHRASE_WORDS)
            .map(|n| window.find(format!("ceremony-word-cell-{n}")).bounds())
            .collect();
        (grid, cells)
    });
    let px = |p: gpui_kit::Pixels| f32::from(p);
    let widest = cells.iter().map(|c| px(c.size.width)).fold(0.0, f32::max);
    for (i, cell) in cells.iter().enumerate() {
        // At least what the longest word needs (8.5rem at 16px), and inside
        // the grid: wrapped to fewer columns, never squeezed or overflowing.
        assert!(
            px(cell.size.width) >= 8.5 * 16. - 1.,
            "cell {} is {} wide",
            i + 1,
            px(cell.size.width)
        );
        assert!(px(cell.origin.x) >= px(grid.origin.x) - 1.);
        assert!(
            px(cell.origin.x) + px(cell.size.width) <= px(grid.origin.x) + px(grid.size.width) + 1.,
            "cell {} overflows the grid",
            i + 1
        );
    }
    assert!(widest > 0.0);
}

#[gpui_kit::test]
fn an_expired_setup_drops_the_typed_words_and_the_passphrase(cx: &mut TestAppContext) {
    let (handle, shell) = offered(cx);
    challenged(cx, handle);
    to_passphrase(cx, handle);
    type_passphrase(cx, handle, &shell, PASSPHRASE, PASSPHRASE);
    let panel = panel(cx, &shell);
    assert!(cx.update(|cx| panel.read(cx).holds_typed_words()));
    advance(cx, u64::try_from(CEREMONY_TTL_MS).unwrap_or(0));
    assert!(has_label(cx, handle, &tr("ceremony.ended.expired")));
    cx.update(|cx| {
        let panel = panel.read(cx);
        assert!(!panel.holds_typed_words(), "the words are gone");
        assert!(panel.passphrase_inputs().iter().all(|i| i.read(cx).value().is_empty()));
    });
    assert_eq!(count(cx, &shell, "account.create.confirm"), 0, "nothing was sent");
}
