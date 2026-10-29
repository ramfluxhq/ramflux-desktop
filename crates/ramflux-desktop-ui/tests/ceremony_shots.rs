// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Offscreen light and dark screenshots of every state of the born-v3
//! ceremonies on the mock source: creating an account with a recovery phrase
//! (name, get ready, the 24 words, the words covered, three words being
//! chosen, the words asked, the passphrase, the result with its networking
//! state, the Escape question and the ways it ends) and recovering one (the
//! entry on the account, the intro, the fingerprint and phrase screen in its
//! states, checking, the result and the ways it ends). The words are the
//! mock's fixed non-production sequence, never a real phrase.
//!
//! Not part of `cargo test`; run explicitly (`RAMFLUX_SHOTS_LOCALE` picks the
//! interface language, `en` by default):
//!
//! ```sh
//! RAMFLUX_SHOTS_DIR=/some/dir RAMFLUX_SHOTS_LOCALE=zh-CN \
//!   cargo test -p ramflux-desktop-ui --test ceremony_shots
//! ```

mod common;

fn main() {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    shots::run();
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    println!("ceremony shots: skipped; GPUI has no headless renderer on this platform");
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod shots {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::Duration;

    use gpui_kit::base::Root;
    use gpui_kit::component::{Theme, ThemeMode};
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{
        AppContext as _, Bounds, Entity, HeadlessAppContext, Point, Window, WindowBounds,
        WindowHandle, WindowOptions, px, size,
    };
    use ramflux_desktop_core::{
        Account, AccountId, AccountKind, CeremonyFailure, DomainEvent, LockState, Networking,
        SecureSessions,
    };
    use ramflux_desktop_test::mock_ceremony::{DEMO_POSITIONS, demo_word};
    use ramflux_desktop_test::mock_recovery::{DEMO_ANCHOR, demo_phrase};
    use ramflux_desktop_ui::AppAssets as Assets;
    use ramflux_desktop_ui::Shell;

    const PASSPHRASE: &str = "river stone lantern";

    struct Run {
        cx: HeadlessAppContext,
        handle: WindowHandle<Root>,
        shell: Entity<Shell>,
        out: PathBuf,
        locale: String,
    }

    impl Run {
        fn open(out: &Path, locale: &str) -> Self {
            let mut cx = HeadlessAppContext::with_platform(
                gpui_kit::platform::current_platform(true).text_system(),
                Arc::new(Assets),
                gpui_kit::platform::current_headless_renderer,
            );
            cx.update(|cx| {
                gpui_kit::init(cx);
                ramflux_desktop_ui::init(cx);
                ramflux_desktop_ui::i18n::select_locale(locale, cx);
            });
            let (handle, shell) = cx
                .update(|cx| {
                    let bounds =
                        Bounds { origin: Point::default(), size: size(px(1100.), px(900.)) };
                    gpui_kit::open_window(
                        WindowOptions {
                            window_bounds: Some(WindowBounds::Windowed(bounds)),
                            focus: false,
                            show: false,
                            ..Default::default()
                        },
                        cx,
                        |window, cx| {
                            cx.new(|cx| Shell::with_scenario("s13_born_v3", true, window, cx))
                        },
                    )
                })
                .expect("offscreen window");
            let handle = handle.downcast::<Root>().expect("Root");
            let mut run =
                Self { cx, handle, shell, out: out.to_path_buf(), locale: locale.to_owned() };
            // A person looks at an active window: the words are drawn.
            run.activate();
            run.advance(2_600);
            run
        }

        fn activate(&mut self) {
            self.cx
                .update_window(self.handle.into(), |_, window, _| window.activate_window())
                .expect("window");
            self.cx.run_until_parked();
        }

        /// What the window losing focus does to the words.
        fn cover_words(&mut self) {
            let panel =
                self.cx.update(|cx| self.shell.read(cx).entry().read(cx).ceremony().clone());
            self.cx.update(|cx| panel.update(cx, |panel, cx| panel.cover_words(cx)));
            self.cx.run_until_parked();
        }

        fn advance(&mut self, ms: u64) {
            let mut left = ms;
            while left > 0 {
                let step = left.min(100);
                self.cx.advance_clock(Duration::from_millis(step));
                self.cx.run_until_parked();
                left -= step;
            }
        }

        fn window<R>(&mut self, f: impl FnOnce(&mut Window, &mut gpui_kit::App) -> R) -> R {
            let result = self
                .cx
                .update_window(self.handle.into(), |_, window, cx| {
                    window.render_frame(cx);
                    f(window, cx)
                })
                .expect("window");
            self.cx.run_until_parked();
            result
        }

        fn click(&mut self, id: &'static str) {
            self.window(move |window, cx| window.click(id, cx));
        }

        fn press(&mut self, key: &'static str) {
            self.window(move |window, cx| window.press(key, cx));
        }

        fn type_into(&mut self, id: &'static str, text: &str) {
            let text = text.to_owned();
            self.window(move |window, cx| {
                window.click(id, cx);
                window.input(&text, cx);
            });
        }

        fn mock<R>(&mut self, f: impl FnOnce(&mut ramflux_desktop_test::MockSource) -> R) -> R {
            let handle = self.cx.update(|cx| self.shell.read(cx).store().clone());
            self.cx.update(|cx| handle.update(cx, |h, _| f(h.mock_mut().expect("a mock"))))
        }

        fn jump(&mut self, ms: u64) {
            let handle = self.cx.update(|cx| self.shell.read(cx).store().clone());
            self.cx.update(|cx| handle.update(cx, |h, cx| h.advance(ms, cx)));
            self.advance(100);
        }

        fn shot(&mut self, name: &str) {
            for (mode, suffix) in [(ThemeMode::Light, "light"), (ThemeMode::Dark, "dark")] {
                self.cx.update(|cx| {
                    Theme::change(mode, None, cx);
                    cx.refresh_windows();
                });
                self.window(|_, _| {});
                let image = self.cx.capture_screenshot(self.handle.into()).expect("render");
                let path = self.out.join(format!("{name}_{}_{suffix}.png", self.locale));
                image.save(&path).expect("write png");
                println!("wrote {}", path.display());
            }
            self.cx.update(|cx| {
                Theme::change(ThemeMode::Light, None, cx);
                cx.refresh_windows();
            });
            self.cx.run_until_parked();
        }

        fn type_words(&mut self, words: [&str; 3]) {
            for (id, word) in
                ["ceremony-word-1", "ceremony-word-2", "ceremony-word-3"].into_iter().zip(words)
            {
                self.type_into(id, word);
            }
        }

        fn right_words() -> [&'static str; 3] {
            DEMO_POSITIONS.map(|p| demo_word(usize::from(p)))
        }

        /// To the name step of a new account.
        fn create_entry(&mut self) {
            self.click("account-create-phrase");
            self.advance(100);
        }

        fn begin(&mut self) {
            self.type_into("ceremony-name-input", "Alex");
            self.click("ceremony-continue");
            self.advance(300);
        }

        fn go_phrase(&mut self) {
            self.begin();
            self.click("ceremony-show");
            self.advance(300);
        }

        fn go_words(&mut self) {
            self.go_phrase();
            self.click("ceremony-hide");
            self.advance(300);
        }

        fn go_passphrase(&mut self) {
            self.go_words();
            self.type_words(Self::right_words());
            self.click("ceremony-next");
            self.advance(100);
        }

        fn account(id: &str, lock_state: LockState) -> DomainEvent {
            DomainEvent::AccountUpdated(Account {
                id: AccountId::new(id),
                label: "Alex".to_owned(),
                kind: AccountKind::HumanInteractive,
                node_id: None,
                device_id: None,
                lock_state,
                self_participant_id: None,
                secure_sessions: SecureSessions::Ready,
                networking: Networking::Live,
            })
        }
    }

    /// Every state of creating an account with a recovery phrase.
    fn creation(out: &Path, locale: &str) {
        let mut run = Run::open(out, locale);
        run.shot("c01_entry");
        run.create_entry();
        run.shot("c02_name");
        run.begin();
        run.shot("c03_ready");
        run.click("ceremony-show");
        run.advance(300);
        run.shot("c04_phrase");
        run.cover_words();
        run.advance(100);
        run.shot("c05_phrase_covered");
        run.activate();
        run.press("escape");
        run.advance(100);
        run.shot("c06_cancel_question");
        run.click("ceremony-cancel-no");
        run.click("ceremony-reveal");
        run.click("ceremony-hide");
        run.shot("c07_choosing_words");
        run.advance(300);
        run.shot("c08_words_asked");
        run.type_words(["ab", Run::right_words()[1], "zzzzz"]);
        run.shot("c09_words_hints");
        let panel = run.cx.update(|cx| run.shell.read(cx).entry().read(cx).ceremony().clone());
        run.window(|window, cx| {
            let inputs = panel.read(cx).word_inputs().clone();
            for input in inputs {
                input.update(cx, |input, cx| input.set_value("", window, cx));
            }
        });
        run.type_words(Run::right_words());
        run.click("ceremony-next");
        run.advance(100);
        run.shot("c10_passphrase_empty");
        run.type_into("ceremony-passphrase-input", "short");
        run.type_into("ceremony-passphrase-repeat", "other");
        run.shot("c11_passphrase_mismatch");
        run.window(|window, cx| {
            let inputs: Vec<_> = panel.read(cx).passphrase_inputs().into_iter().cloned().collect();
            for input in inputs {
                input.update(cx, |input, cx| input.set_value("", window, cx));
            }
        });
        run.type_into("ceremony-passphrase-input", PASSPHRASE);
        run.type_into("ceremony-passphrase-repeat", PASSPHRASE);
        run.shot("c12_passphrase_ready");
        run.click("ceremony-confirm-submit");
        run.advance(300);
        run.shot("c13_done_pending");
        run.click("ceremony-done-close");
        run.advance(300);
        run.shot("c14_workspace_pending");

        // The ways a ceremony ends.
        let mut run = Run::open(out, locale);
        run.create_entry();
        run.begin();
        run.click("ceremony-cancel");
        run.advance(300);
        run.shot("c15_ended_cancelled");
        let mut run = Run::open(out, locale);
        run.create_entry();
        run.go_words();
        run.type_words(["abandon", "ability", "able"]);
        run.click("ceremony-next");
        run.type_into("ceremony-passphrase-input", PASSPHRASE);
        run.type_into("ceremony-passphrase-repeat", PASSPHRASE);
        run.click("ceremony-confirm-submit");
        run.advance(300);
        run.shot("c16_ended_mismatch");
        let mut run = Run::open(out, locale);
        run.create_entry();
        run.go_phrase();
        run.jump(300_000);
        run.shot("c17_ended_expired");
        let mut run = Run::open(out, locale);
        run.mock(|m| m.ceremony_mut().refuse_next_begin(CeremonyFailure::VaultUnavailable));
        run.create_entry();
        run.begin();
        run.shot("c18_ended_vault");
        let mut live = Run::open(out, locale);
        live.mock(|m| m.ceremony_mut().set_networking(Networking::Live));
        live.create_entry();
        live.go_passphrase();
        live.type_into("ceremony-passphrase-input", PASSPHRASE);
        live.type_into("ceremony-passphrase-repeat", PASSPHRASE);
        live.click("ceremony-confirm-submit");
        live.advance(300);
        live.shot("c19_done_live");
    }

    /// Every state of recovering an account from its phrase.
    fn recovery(out: &Path, locale: &str) {
        let world = |out: &Path, locale: &str| {
            let mut run = Run::open(out, locale);
            run.mock(|m| {
                m.ceremony_mut().recovery_mut().allow("acct-r", "Alex", PASSPHRASE);
                m.inject(0, Run::account("acct-r", LockState::Locked));
            });
            run.advance(300);
            run
        };
        let mut run = world(out, locale);
        run.shot("r01_account_entry");
        run.click("account-recover-acct-r");
        run.advance(100);
        run.shot("r02_intro");
        run.click("recovery-continue");
        run.shot("r03_intro_waiting");
        run.advance(300);
        run.shot("r04_check_empty");
        let grouped = DEMO_ANCHOR
            .chars()
            .collect::<Vec<_>>()
            .chunks(4)
            .map(|g| g.iter().collect::<String>())
            .collect::<Vec<_>>()
            .join(" ");
        run.type_into("recovery-fingerprint-input", "k3J9 xQ2m");
        run.shot("r05_fingerprint_mismatch");
        let panel = run.cx.update(|cx| run.shell.read(cx).entry().read(cx).recovery().clone());
        let set = |run: &mut Run, index: usize, text: String| {
            let panel = panel.clone();
            run.window(move |window, cx| {
                let input = panel.read(cx).inputs()[index].clone();
                input.update(cx, |input, cx| input.set_value(text, window, cx));
            });
        };
        set(&mut run, 0, grouped.clone());
        let ten = (1..=10).map(demo_word).collect::<Vec<_>>().join(" ");
        set(&mut run, 1, format!("{ten} xqzvw "));
        run.shot("r06_phrase_unknown_word");
        set(&mut run, 1, ten);
        run.shot("r07_phrase_counting");
        set(&mut run, 1, demo_phrase());
        run.type_into("recovery-passphrase-input", PASSPHRASE);
        run.shot("r08_ready_to_recover");
        run.click("recovery-submit");
        run.shot("r09_checking");
        run.advance(300);
        run.shot("r10_done");

        let mut run = world(out, locale);
        run.click("account-recover-acct-r");
        run.click("recovery-continue");
        run.advance(300);
        let wrong = (1..=24).map(|n| demo_word(n + 1)).collect::<Vec<_>>().join(" ");
        run.type_into("recovery-fingerprint-input", &grouped);
        run.type_into("recovery-phrase-input", &wrong);
        run.type_into("recovery-passphrase-input", PASSPHRASE);
        run.click("recovery-submit");
        run.advance(300);
        run.shot("r11_ended_mismatch");
        let mut run = world(out, locale);
        run.click("account-recover-acct-r");
        run.click("recovery-continue");
        run.advance(300);
        run.type_into("recovery-fingerprint-input", &grouped);
        run.type_into("recovery-phrase-input", &demo_phrase());
        run.type_into("recovery-passphrase-input", "not the passphrase");
        run.click("recovery-submit");
        run.advance(300);
        run.shot("r12_ended_invalid");
        let mut run = world(out, locale);
        run.click("account-recover-acct-r");
        run.click("recovery-continue");
        run.advance(300);
        run.jump(300_000);
        run.shot("r13_ended_expired");
        let mut run = world(out, locale);
        run.click("account-recover-acct-r");
        run.click("recovery-continue");
        run.advance(300);
        run.click("recovery-cancel");
        run.advance(300);
        run.shot("r14_ended_cancelled");
    }

    pub fn run() {
        let out = std::env::var_os("RAMFLUX_SHOTS_DIR").map_or_else(
            || PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("ceremony-shots"),
            PathBuf::from,
        );
        let locale = std::env::var("RAMFLUX_SHOTS_LOCALE").unwrap_or_else(|_| "en".to_owned());
        std::fs::create_dir_all(&out).expect("output directory");
        creation(&out, &locale);
        recovery(&out, &locale);
    }
}
