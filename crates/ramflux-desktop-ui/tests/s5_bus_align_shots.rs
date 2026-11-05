// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Explicit offscreen evidence of six bus-alignment states, in both themes.
//! Set RAMFLUX_SHOTS_DIR and RAMFLUX_SHOTS_LOCALE (en or zh-CN).
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn main() {
    shots::run();
}
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn main() {
    panic!("headless renderer required for bus alignment screenshots");
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod shots {
    use gpui_kit::base::Root;
    use gpui_kit::component::{Theme, ThemeMode};
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{
        AppContext as _, Bounds, Entity, HeadlessAppContext, Point, WindowBounds, WindowHandle,
        WindowOptions, px, size,
    };
    use ramflux_desktop_core::{
        AccountId, ContextId, Intent, IntentKind, MomentKind, Networking, Receipt, SyncMode,
    };
    use ramflux_desktop_sdk::{SdkConfig, SdkHub};
    use ramflux_desktop_test::fake_bus::{FakeBus, Reply, socket_path};
    use ramflux_desktop_ui::{
        AppAssets, Shell,
        shell::{ShellOptions, SourceChoice},
    };
    use serde_json::{Value, json};
    use std::{
        path::PathBuf,
        sync::Arc,
        time::{Duration, Instant},
    };

    const STATES: [&str; 6] =
        ["connecting", "node_busy", "live", "waiting_for_node", "failed", "degraded_stream"];

    struct Run {
        cx: HeadlessAppContext,
        window: WindowHandle<Root>,
        shell: Entity<Shell>,
        out: PathBuf,
        locale: String,
    }
    impl Run {
        fn open(out: PathBuf, locale: String, config: SdkConfig, hub: &SdkHub) -> Self {
            let mut cx = HeadlessAppContext::with_platform(
                gpui_kit::platform::current_platform(true).text_system(),
                Arc::new(AppAssets),
                gpui_kit::platform::current_headless_renderer,
            );
            cx.update(|cx| {
                gpui_kit::init(cx);
                ramflux_desktop_ui::init(cx);
                ramflux_desktop_ui::i18n::select_locale(&locale, cx);
            });
            let mut options = ShellOptions::for_scenario("", true);
            options.source = SourceChoice::Rfd(config);
            let (window, shell) = cx
                .update(|cx| {
                    gpui_kit::open_window(
                        WindowOptions {
                            window_bounds: Some(WindowBounds::Windowed(Bounds {
                                origin: Point::default(),
                                size: size(px(1280.), px(860.)),
                            })),
                            focus: false,
                            show: false,
                            ..Default::default()
                        },
                        cx,
                        |window, cx| cx.new(|cx| Shell::with_options(options, window, cx)),
                    )
                })
                .expect("offscreen window");
            let window = window.downcast::<Root>().expect("root");
            cx.update(|cx| {
                let store = shell.read(cx).store().clone();
                store.update(cx, |s, cx| s.attach_live(hub.clone(), cx));
            });
            Self { cx, window, shell, out, locale }
        }
        fn tick(&mut self) {
            std::thread::sleep(Duration::from_millis(10));
            self.cx.advance_clock(Duration::from_millis(50));
            self.cx.run_until_parked();
        }
        fn until(&mut self, what: &str, mut done: impl FnMut(&mut Self) -> bool) {
            let limit = Instant::now() + Duration::from_secs(20);
            while !done(self) {
                assert!(Instant::now() < limit, "timed out: {what}");
                self.tick();
            }
        }
        fn store<R>(&mut self, f: impl FnOnce(&ramflux_desktop_core::Store) -> R) -> R {
            self.cx.update(|cx| f(self.shell.read(cx).store().read(cx).store()))
        }
        fn show(&mut self) {
            let workspace = self.cx.update(|cx| self.shell.read(cx).workspace().clone());
            self.cx
                .update_window(self.window.into(), |_, window, cx| {
                    workspace.update(cx, |w, cx| {
                        w.select_context(Some(ContextId::new("dm-ana")), window, cx)
                    })
                })
                .expect("workspace");
        }
        fn waiting(&mut self) -> bool {
            self.store(|s| s.moments().any(|m| matches!(&m.kind, MomentKind::Message(p) if matches!(p.receipt, Receipt::WaitingForNode { .. }))))
        }
        fn shot(&mut self, state: &str) {
            for (mode, suffix) in [(ThemeMode::Light, "light"), (ThemeMode::Dark, "dark")] {
                self.cx.update(|cx| {
                    Theme::change(mode, None, cx);
                    cx.refresh_windows();
                });
                self.cx
                    .update_window(self.window.into(), |_, window, cx| window.render_frame(cx))
                    .expect("frame");
                self.cx.run_until_parked();
                let image = self.cx.capture_screenshot(self.window.into()).expect("render");
                let path = self.out.join(format!("{state}_{}_{suffix}.png", self.locale));
                image.save(&path).expect("png");
                println!("wrote {}", path.display());
            }
        }
    }
    pub fn run() {
        let out = std::env::var_os("RAMFLUX_SHOTS_DIR")
            .map(PathBuf::from)
            .expect("RAMFLUX_SHOTS_DIR required");
        let locale = std::env::var("RAMFLUX_SHOTS_LOCALE").unwrap_or_else(|_| "en".into());
        assert!(matches!(locale.as_str(), "en" | "zh-CN"));
        std::fs::create_dir_all(&out).expect("output directory");
        // Remove only this program's expected outputs, so a missing state can
        // never pass by reusing a PNG from an earlier run.
        for state in STATES {
            for theme in ["light", "dark"] {
                let _ = std::fs::remove_file(out.join(format!("{state}_{locale}_{theme}.png")));
            }
        }
        let path = socket_path(&format!("s5-shots-{locale}"));
        let frames: std::collections::BTreeMap<String, Value> = serde_json::from_str(include_str!(
            "../../ramflux-desktop-sdk/tests/fixtures/rfd_frames.json"
        ))
        .expect("frames");
        let fake = FakeBus::start(&path, move |method, _, _| {
            if method == "message.submit" {
                return Reply::Refused {
                    code: "NetworkingPending",
                    message: "offline".into(),
                    reason_code: "pending",
                    details: Some(json!({"networking":{"state":"networking_pending"}})),
                };
            }
            let mut body = frames.get(method).cloned().unwrap_or(json!({}));
            if method == "account.status" {
                body["networking_snapshot"] = json!({"state":"networking_pending","phase":"connecting","generation":1,"cause":null});
            }
            Reply::Ok(body)
        });
        fake.enable_stream();
        let mut config = SdkConfig::new(path);
        config.call_api = false;
        config.backup_api = false;
        config.pause_test_time = true;
        let hub = SdkHub::start(config.clone()).expect("hub");
        let mut run = Run::open(out.clone(), locale.clone(), config, &hub);
        run.until("workspace", |r| {
            r.cx.update(|cx| r.shell.read(cx).page()) == ramflux_desktop_ui::route::Page::Workspace
                && r.store(|s| s.context(&ContextId::new("dm-ana")).is_some())
        });
        run.show();
        // Capture the actual unnamed, paused attachment after its directory
        // has arrived, rather than a partially loaded row.
        run.until("paused attachment fixture", |r| {
            r.store(|s| {
                s.object(&ramflux_desktop_core::ObjectId::new("attachment:m2:0"))
                    .is_some_and(|o| o.name.is_none()
                        && matches!(&o.transfer, ramflux_desktop_core::TransferState::Failed { reason } if reason == "paused"))
                    && s.moments().any(|m| matches!(&m.kind, MomentKind::Message(p)
                        if p.attachment.as_ref().is_some_and(|id| id.as_str() == "attachment:m2:0")))
            })
        });
        run.until("stream", |_| fake.streaming("acct-main"));
        run.until("connecting", |r| {
            r.store(|s| {
                s.account(&AccountId::new("acct-main"))
                    .is_some_and(|a| a.networking == Networking::Pending)
            })
        });
        run.shot("connecting");
        let event = |reason: Value, state: &str| json!({"account_id":"acct-main","state":"live","networking_reason":reason,"session":{"state":state,"phase":"backoff","generation":2,"cause":null}});
        fake.publish(
            "acct-main",
            "account.networking.changed",
            event(json!({"kind":"node_busy","retry_after_ms":5000}), "networking_pending"),
        );
        run.until("node busy", |r| {
            r.store(|s| {
                s.account(&AccountId::new("acct-main"))
                    .is_some_and(|a| a.networking_reason.is_some())
            })
        });
        run.shot("node_busy");
        fake.publish("acct-main", "account.networking.changed", event(Value::Null, "live"));
        run.until("live", |r| {
            r.store(|s| {
                s.account(&AccountId::new("acct-main"))
                    .is_some_and(|a| a.networking == Networking::Live)
            })
        });
        run.shot("live");
        run.cx.update(|cx| {
            let store = run.shell.read(cx).store().clone();
            store.update(cx, |s, cx| {
                s.dispatch(
                    Intent::human(IntentKind::SendMessage {
                        context_id: ContextId::new("dm-ana"),
                        text: "A message waiting for the node".into(),
                    }),
                    cx,
                );
            });
        });
        run.until("waiting", Run::waiting);
        run.shot("waiting_for_node");
        hub.advance_test_time(Duration::from_secs(600));
        run.until("failed", |r| r.store(|s| s.moments().any(|m| m.lifecycle == ramflux_desktop_core::Lifecycle::Failed && matches!(&m.kind, MomentKind::Message(p) if p.receipt == (Receipt::Failed { retryable: true })))));
        run.shot("failed");
        fake.disable_stream();
        fake.end_streams("acct-main", false);
        run.until("degraded stream", |r| {
            r.store(|s| s.sync_status().is_some_and(|s| s.mode != SyncMode::Stream))
        });
        run.cx.update(|cx| {
            run.shell.update(cx, |s, cx| {
                s.open_settings_at(ramflux_desktop_ui::settings::SettingsSection::Diagnostics, cx)
            })
        });
        run.cx.run_until_parked();
        run.cx
            .update_window(run.window.into(), |_, window, cx| {
                window.render_frame(cx);
                assert!(
                    !window.find_all("diagnostics-preview").is_empty(),
                    "degraded stream diagnostics must be rendered"
                );
            })
            .expect("diagnostics");
        run.shot("degraded_stream");
        fake.shutdown();
        for state in STATES {
            for theme in ["light", "dark"] {
                let path = out.join(format!("{state}_{locale}_{theme}.png"));
                assert!(
                    std::fs::metadata(&path).is_ok_and(|m| m.len() > 0),
                    "missing or empty required screenshot: {}",
                    path.display()
                );
            }
        }
        println!("S5_SHOTS: 12 nonempty images; locale={locale}; states=6; themes=2");
    }
}
