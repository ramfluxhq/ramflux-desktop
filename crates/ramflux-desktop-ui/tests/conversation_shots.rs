// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Offscreen light and dark screenshots of starting a conversation and of
//! the attach states, on a fake daemon (see `sdk_fake`), with the developer
//! panel closed.
//!
//! Not part of `cargo test`; run explicitly:
//!
//! ```sh
//! RAMFLUX_SHOTS_DIR=/some/dir cargo test -p ramflux-desktop-ui --test conversation_shots
//! ```

mod common;
mod sdk_fake;

fn main() {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    shots::run();
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    println!("conversation shots: skipped; GPUI has no headless renderer on this platform");
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod shots {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use gpui_kit::assets::Assets;
    use gpui_kit::base::Root;
    use gpui_kit::component::{Theme, ThemeMode};
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{
        AppContext as _, Bounds, Entity, HeadlessAppContext, Point, Window, WindowBounds,
        WindowHandle, WindowOptions, px, size,
    };
    use ramflux_desktop_core::{ContextId, ParticipantId};
    use ramflux_desktop_ui::Shell;
    use ramflux_desktop_ui::route::Page;
    use ramflux_desktop_ui::settings::SettingsSection;

    use crate::sdk_fake;

    struct Run {
        cx: HeadlessAppContext,
        handle: WindowHandle<Root>,
        shell: Entity<Shell>,
        out: PathBuf,
    }

    impl Run {
        fn tick(&mut self) {
            std::thread::sleep(Duration::from_millis(10));
            self.cx.advance_clock(Duration::from_millis(50));
            self.cx.run_until_parked();
        }

        fn until(&mut self, what: &str, mut done: impl FnMut(&mut Self) -> bool) {
            let limit = Instant::now() + Duration::from_secs(30);
            while !done(self) {
                assert!(Instant::now() < limit, "timed out waiting for {what}");
                self.tick();
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

        fn click(&mut self, scope: Option<&str>, id: &str) {
            let (scope, id) = (scope.map(str::to_owned), id.to_owned());
            self.window(move |window, cx| match scope {
                Some(scope) => window.within(scope).click(id, cx),
                None => window.click(id, cx),
            });
        }

        fn type_into(&mut self, id: &str, text: &str) {
            let (id, text) = (id.to_owned(), text.to_owned());
            self.window(move |window, cx| {
                window.click(id.clone(), cx);
                window.press("secondary-a", cx);
                window.press("backspace", cx);
                window.input(&text, cx);
            });
        }

        fn store<R>(&mut self, f: impl FnOnce(&ramflux_desktop_core::Store) -> R) -> R {
            self.cx.update(|cx| f(self.shell.read(cx).store().read(cx).store()))
        }

        fn workspace(&mut self) -> Entity<ramflux_desktop_ui::workspace::Workspace> {
            self.cx.update(|cx| self.shell.read(cx).workspace().clone())
        }

        fn open_contacts_row(&mut self, who: &str) {
            let row = format!("contacts-{who}");
            if !self.window(move |window, _| window.try_find(row).is_some()) {
                self.click(None, "rail-contacts");
            }
            self.click(Some(&format!("contacts-{who}")), "message");
        }

        fn close_dock(&mut self) {
            if self.window(|window, _| window.try_find("dock-content-contacts").is_some()) {
                self.click(None, "rail-contacts");
            }
        }

        fn shot(&mut self, name: &str) {
            let dev = self.cx.update(|cx| self.shell.read(cx).is_dev_panel_visible());
            assert!(!dev, "screenshots are taken with the developer panel closed");
            for (mode, suffix) in [(ThemeMode::Light, "light"), (ThemeMode::Dark, "dark")] {
                self.cx.update(|cx| {
                    Theme::change(mode, None, cx);
                    cx.refresh_windows();
                });
                self.window(|_, _| {});
                let image = self.cx.capture_screenshot(self.handle.into()).expect("render");
                let path = self.out.join(format!("{name}_{suffix}.png"));
                image.save(&path).expect("write png");
                println!("wrote {}", path.display());
            }
            self.cx.update(|cx| {
                Theme::change(ThemeMode::Light, None, cx);
                cx.refresh_windows();
            });
            self.cx.run_until_parked();
        }
    }

    fn scratch_file(dir: &Path, name: &str, len: usize) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, vec![b'r'; len]).expect("file");
        path
    }

    pub fn run() {
        let out = std::env::var_os("RAMFLUX_SHOTS_DIR").map_or_else(
            || PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("conversation-shots"),
            PathBuf::from,
        );
        std::fs::create_dir_all(&out).expect("output directory");
        let files = std::env::temp_dir().join(format!("rf-f10-shots-{}", std::process::id()));
        std::fs::create_dir_all(&files).expect("scratch");
        let (fake, state, socket) = sdk_fake::start("f10-shots");
        let mut cx = HeadlessAppContext::with_platform(
            gpui_kit::platform::current_platform(true).text_system(),
            Arc::new(Assets),
            gpui_kit::platform::current_headless_renderer,
        );
        cx.update(|cx| {
            gpui_kit::init(cx);
            ramflux_desktop_ui::init(cx);
        });
        let options = sdk_fake::options(&socket);
        let (handle, shell) = cx
            .update(|cx| {
                let bounds = Bounds { origin: Point::default(), size: size(px(1280.), px(860.)) };
                gpui_kit::open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(bounds)),
                        focus: false,
                        show: false,
                        ..Default::default()
                    },
                    cx,
                    |window, cx| cx.new(|cx| Shell::with_options(options, window, cx)),
                )
            })
            .expect("offscreen window");
        let handle = handle.downcast::<Root>().expect("Root");
        let mut run = Run { cx, handle, shell, out };
        run.until("the workspace", |r| {
            r.cx.update(|cx| r.shell.read(cx).page()) == Page::Workspace
        });
        run.until("contacts", |r| {
            r.store(|s| {
                s.participant(&ParticipantId::new("principal-dee")).is_some()
                    && s.context(&ContextId::new("dm-ana")).is_some()
            })
        });

        // Starting a conversation.
        run.open_contacts_row("principal-bo");
        run.shot("f10_01_new_conversation_contacts");
        run.close_dock();
        run.type_into("composer-input", "Hello Bo, a first message");
        run.shot("f10_02_new_conversation_draft");
        run.open_contacts_row("principal-cy");
        run.close_dock();
        run.shot("f10_03_new_conversation_key_changed");
        run.open_contacts_row("principal-dee");
        run.close_dock();
        run.type_into("composer-input", "Hello Dee");
        run.click(None, "composer-send");
        run.until("the refusal", |r| {
            r.store(|s| s.failures().iter().any(|f| f.method == "conversation.start"))
        });
        run.tick();
        run.shot("f10_04_new_conversation_unverified_key");
        run.open_contacts_row("principal-bo");
        run.close_dock();
        run.type_into("composer-input", "Hello Bo, a first message");
        run.click(None, "composer-send");
        run.until("the started conversation", |r| {
            let workspace = r.workspace();
            r.cx.update(|cx| workspace.read(cx).current_context().is_some())
        });
        for _ in 0..10 {
            run.tick();
        }
        run.shot("f10_05_conversation_started");
        run.click(None, "stack-new");
        run.shot("f10_06_stack_new_menu");
        run.window(|window, cx| window.press("escape", cx));

        // Attaching.
        let workspace = run.workspace();
        let ana = ContextId::new("dm-ana");
        run.window(move |window, cx| {
            workspace.update(cx, |w, cx| w.select_context(Some(ana), window, cx));
        });
        let composer =
            run.cx.update(|cx| run.shell.read(cx).workspace().read(cx).composer().clone());
        let attach = |run: &mut Run, path: &Path| {
            run.click(None, "composer-attach");
            let chosen = path.display().to_string();
            let composer = composer.clone();
            run.window(move |window, cx| {
                composer.update(cx, |c, cx| c.attach_chosen(&chosen, window, cx));
            });
        };
        let report = scratch_file(&files, "quarterly-report.txt", 8192);
        attach(&mut run, &report);
        run.shot("f10_07_attach_staged");
        let huge =
            scratch_file(&files, "raw-footage.mov", ramflux_desktop_sdk::ATTACHMENT_LIMIT + 4096);
        attach(&mut run, &huge);
        run.shot("f10_08_attach_too_large");
        attach(&mut run, &report);
        state.lock().expect("state").submit_delay = Duration::from_secs(3);
        run.click(None, "composer-send");
        run.until("the message on its way", |r| {
            r.store(|s| s.objects().any(|o| o.name.as_deref() == Some("quarterly-report.txt")))
        });
        run.shot("f10_09_attach_sending");
        run.until("the sent attachment", |r| {
            r.store(|s| {
                s.objects().any(|o| {
                    o.name.as_deref() == Some("quarterly-report.txt")
                        && o.transfer == ramflux_desktop_core::TransferState::Complete
                })
            })
        });
        for _ in 0..10 {
            run.tick();
        }
        run.shot("f10_10_attach_sent");
        {
            let mut state = state.lock().expect("state");
            state.submit_delay = Duration::ZERO;
            state.fail_submit = Some("local bus error: relay object upload failed".to_owned());
        }
        let broken = scratch_file(&files, "notes.txt", 512);
        attach(&mut run, &broken);
        run.click(None, "composer-send");
        run.until("the failed attachment", |r| r.store(|s| !s.failures().is_empty()));
        for _ in 0..5 {
            run.tick();
        }
        run.shot("f10_11_attach_failed");

        // Settings › Diagnostics (this fake offers no event stream: polling).
        let shell = run.shell.clone();
        run.cx.update(|cx| {
            shell.update(cx, |s, cx| s.open_settings_at(SettingsSection::Diagnostics, cx));
        });
        run.tick();
        run.shot("f10_12_diagnostics_sync");

        fake.shutdown();
        let _ = std::fs::remove_dir_all(files);
    }
}
