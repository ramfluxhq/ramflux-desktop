// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Offscreen light and dark screenshots, in English and Simplified Chinese,
//! of agent surfaces that come from a daemon through the SDK source (a fake
//! daemon holding them on the protocol's state machine and answering as the
//! pinned daemon does, see `sdk_fake`), with the developer panel closed: a
//! reply mid-stream and then final, a surface out of sync with its author,
//! the daemon's typed refusal of an update request (rate limited), the sent
//! request, the author's state, a stale action the daemon refused, and the
//! renderer limits and version in Settings › Diagnostics.
//!
//! Not part of `cargo test`; run explicitly:
//!
//! ```sh
//! RAMFLUX_SHOTS_DIR=/some/dir cargo test -p ramflux-desktop-ui --test surface_shots
//! ```

mod common;
mod sdk_fake;

fn main() {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    shots::run();
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    println!("surface shots: skipped; GPUI has no headless renderer on this platform");
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod shots {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use gpui_kit::base::Root;
    use gpui_kit::component::{Theme, ThemeMode};
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{
        AppContext as _, Bounds, Entity, HeadlessAppContext, Point, ScrollDelta, Window,
        WindowBounds, WindowHandle, WindowOptions, point, px, size,
    };
    use ramflux_desktop_core::{ContextId, SurfaceId, SurfaceRecord, SyncMode};
    use ramflux_desktop_ui::AppAssets as Assets;
    use ramflux_desktop_ui::Shell;
    use ramflux_desktop_ui::route::Page;
    use ramflux_desktop_ui::settings::SettingsSection;

    use crate::sdk_fake;
    use crate::sdk_fake::author::{Bot, GROUP, scenario_surfaces};

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

        fn record(&mut self, reference: &str) -> Option<SurfaceRecord> {
            self.cx.update(|cx| {
                let store = self.shell.read(cx).store().read(cx).store();
                store
                    .surface_moment(&SurfaceId::new(reference))
                    .and_then(|m| store.surface(m))
                    .cloned()
            })
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

        fn click(&mut self, scope: &str, id: &str) {
            let (scope, id) = (scope.to_owned(), id.to_owned());
            self.window(move |window, cx| window.within(scope).click(id, cx));
        }

        fn shot(&mut self, name: &str) {
            let dev = self.cx.update(|cx| self.shell.read(cx).is_dev_panel_visible());
            assert!(!dev, "screenshots are taken with the developer panel closed");
            for (locale, tag) in [("en", "en"), ("zh-CN", "zh")] {
                for (mode, suffix) in [(ThemeMode::Light, "light"), (ThemeMode::Dark, "dark")] {
                    self.cx.update(|cx| {
                        ramflux_desktop_ui::i18n::select_locale(locale, cx);
                        Theme::change(mode, None, cx);
                        cx.refresh_windows();
                    });
                    self.window(|_, _| {});
                    let image = self.cx.capture_screenshot(self.handle.into()).expect("render");
                    let path = self.out.join(format!("{name}_{tag}_{suffix}.png"));
                    image.save(&path).expect("write png");
                    println!("wrote {}", path.display());
                }
            }
            self.cx.update(|cx| {
                ramflux_desktop_ui::i18n::select_locale("en", cx);
                Theme::change(ThemeMode::Light, None, cx);
                cx.refresh_windows();
            });
            self.cx.run_until_parked();
        }
    }

    pub fn run() {
        let out = std::env::var_os("RAMFLUX_SHOTS_DIR").map_or_else(
            || PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("surface-shots"),
            PathBuf::from,
        );
        std::fs::create_dir_all(&out).expect("output directory");
        let (fake, state, socket) = sdk_fake::start("f13a-shots");
        fake.enable_stream();
        let mut cx = HeadlessAppContext::with_platform(
            gpui_kit::platform::current_platform(true).text_system(),
            Arc::new(Assets),
            gpui_kit::platform::current_headless_renderer,
        );
        cx.update(|cx| {
            gpui_kit::init(cx);
            ramflux_desktop_ui::init(cx);
            // Shots start in English.
            ramflux_desktop_ui::i18n::select_locale("en", cx);
        });
        let options = sdk_fake::options(&socket);
        let (handle, shell) = cx
            .update(|cx| {
                let bounds = Bounds { origin: Point::default(), size: size(px(1280.), px(900.)) };
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
        run.until("the stream", |r| {
            r.cx.update(|cx| {
                r.shell.read(cx).store().read(cx).store().sync_status().map(|s| s.mode)
            }) == Some(SyncMode::Stream)
        });
        let workspace = run.cx.update(|cx| run.shell.read(cx).workspace().clone());
        run.window(move |window, cx| {
            workspace.update(cx, |w, cx| w.select_context(Some(ContextId::new(GROUP)), window, cx));
        });
        let mut bot = Bot::new();

        // The reply mid-stream (the daemon holds version 3, stream open).
        let s10 = scenario_surfaces("s10_ai_participant");
        let (reply, _) = bot.send(&fake, &state, &s10[0], true);
        run.until("the reply", |r| r.record(&reply).is_some());
        for event in &s10[1..3] {
            assert!(bot.send(&fake, &state, event, true).1);
            let version = event.version();
            run.until("an append", |r| r.record(&reply).is_some_and(|x| x.version == version));
        }
        run.tick();
        run.shot("f13a_01_surface_streaming");

        // The final append: the daemon reports the stream final, the mark
        // ends; the header shows when the author last changed it.
        bot.send(&fake, &state, &s10[3], true);
        run.until("final", |r| r.record(&reply).is_some_and(|x| x.version == 4));
        run.tick();
        run.shot("f13a_02_surface_final");

        // A choice the daemon then knows is out of sync.
        let s11 = scenario_surfaces("s11_surface_versions");
        let (pick, _) = bot.send(&fake, &state, &s11[0], true);
        run.until("the choice", |r| r.record(&pick).is_some());
        let surfaces = state.lock().expect("state").surfaces.clone();
        surfaces.set_out_of_sync(&pick, true);
        Bot::announce(&fake, &state, &pick);
        run.until("out of sync", |r| {
            r.record(&pick)
                .is_some_and(|x| x.state == ramflux_desktop_core::SurfaceState::OutOfSync)
        });
        run.tick();
        run.shot("f13a_03_surface_out_of_sync");

        // Asked for in this window already: the daemon's rate_limited answer.
        let scope = format!("moment-surface:{GROUP}:{pick}");
        surfaces.asked_recently(&pick);
        run.click(&scope, "surface-request-update");
        run.until("the refusal", |r| r.record(&pick).is_some_and(|x| x.update.is_some()));
        run.tick();
        run.shot("f13a_04_surface_update_rate_limited");

        surfaces.next_rate_window();
        run.click(&scope, "surface-request-update");
        run.until("sent", |r| {
            r.record(&pick)
                .is_some_and(|x| x.update == Some(ramflux_desktop_core::UpdateRequest::Sent))
        });
        run.tick();
        run.shot("f13a_05_surface_update_requested");

        // The author's current state arrives: in sync again.
        surfaces.set_out_of_sync(&pick, false);
        bot.send(&fake, &state, &s11[1], true);
        run.until("in sync", |r| {
            r.record(&pick).is_some_and(|x| x.state == ramflux_desktop_core::SurfaceState::Open)
        });
        run.tick();
        run.shot("f13a_06_surface_resynced");

        // The author changes it before this window reads it; the user acts
        // on v2: the daemon refuses it (stale_version), review again.
        bot.send(&fake, &state, &s11[3], false);
        run.click(&scope, "action-thu");
        let moment = ramflux_desktop_core::MomentId::new(format!("surface:{GROUP}:{pick}"));
        let workspace = run.cx.update(|cx| run.shell.read(cx).workspace().clone());
        run.until("the review notice", |r| {
            r.cx.update(|cx| workspace.read(cx).conversation().read(cx).is_review_again(&moment))
        });
        run.tick();
        run.shot("f13a_07_surface_stale_action");

        let shell = run.shell.clone();
        run.cx.update(|cx| {
            shell.update(cx, |s, cx| s.open_settings_at(SettingsSection::Diagnostics, cx));
        });
        run.tick();
        run.shot("f13a_08_diagnostics_limits");

        // Scroll down to the version line, at the
        // lowest anchor shown (the wheel lands on its center).
        for _ in 0..12 {
            run.window(|window, cx| {
                let bottom = window.viewport_size().height;
                let anchors = [
                    "diagnostics-preview",
                    "renderer-profile",
                    "launch-socket",
                    "launch-save",
                    "launch-daemon-help",
                ];
                let anchor = anchors.into_iter().rev().find(|id| {
                    window
                        .try_find(*id)
                        .is_some_and(|e| e.visible() && e.bounds().center().y < bottom)
                });
                if let Some(anchor) = anchor {
                    let delta = ScrollDelta::Pixels(point(px(0.), px(-300.)));
                    window.scroll(anchor, delta, cx);
                }
            });
            run.tick();
        }
        run.shot("f13a_09_diagnostics_version");
        fake.shutdown();
    }
}
