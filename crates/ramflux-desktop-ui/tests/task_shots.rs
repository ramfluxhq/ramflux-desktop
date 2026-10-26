// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Offscreen screenshots of tasks that come from a daemon through the SDK
//! source (a fake daemon holding them on the protocol's state machine, see
//! `sdk_fake`), light and dark, in English and Chinese, with the developer
//! panel closed: a running task, a step that needs the user's approval, an
//! assignee that stalled, the stalled task cancelled by its requester, an
//! approval whose delivery failed (with Retry), and the approved task done.
//!
//! Not part of `cargo test`; run explicitly:
//!
//! ```sh
//! RAMFLUX_SHOTS_DIR=/some/dir cargo test -p ramflux-desktop-ui --test task_shots
//! ```

mod common;
mod sdk_fake;

fn main() {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    shots::run();
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    println!("task shots: skipped; GPUI has no headless renderer on this platform");
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod shots {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use gpui_kit::base::Root;
    use gpui_kit::component::{Theme, ThemeMode};
    use gpui_kit::test::{TestEventExt as _, TestWindowExt as _};
    use gpui_kit::{
        AppContext as _, Bounds, Entity, HeadlessAppContext, Point, Window, WindowBounds,
        WindowHandle, WindowOptions, px, size,
    };
    use ramflux_desktop_core::{ContextId, GrantId, SyncMode, Task, TaskId, TaskState};
    use ramflux_desktop_sdk::translate::tasks::{task_key, task_moment_id};
    use ramflux_desktop_test::fake_tasks::{approval_step, task_step};
    use ramflux_desktop_ui::AppAssets as Assets;
    use ramflux_desktop_ui::Shell;
    use ramflux_desktop_ui::route::Page;
    use ramflux_protocol::TaskStatus;
    use serde_json::json;

    use crate::sdk_fake::{self, BOT, GROUP};

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

        fn task(&mut self, id: &TaskId) -> Option<Task> {
            self.cx.update(|cx| self.shell.read(cx).store().read(cx).store().task(id).cloned())
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

        fn click(&mut self, scope: String, id: &'static str) {
            self.window(move |window, cx| window.within(scope).click(id, cx));
        }

        /// Light and dark, in English and in Chinese.
        fn shot(&mut self, name: &str) {
            let dev = self.cx.update(|cx| self.shell.read(cx).is_dev_panel_visible());
            assert!(!dev, "screenshots are taken with the developer panel closed");
            for locale in ["en", "zh-CN"] {
                self.cx.update(|cx| ramflux_desktop_ui::i18n::select_locale(locale, cx));
                for (mode, suffix) in [(ThemeMode::Light, "light"), (ThemeMode::Dark, "dark")] {
                    self.cx.update(|cx| {
                        Theme::change(mode, None, cx);
                        cx.refresh_windows();
                    });
                    // Let transitions (the progress bar) settle.
                    for _ in 0..20 {
                        self.tick();
                    }
                    self.window(|_, _| {});
                    let image = self.cx.capture_screenshot(self.handle.into()).expect("render");
                    let path = self.out.join(format!("{name}_{locale}_{suffix}.png"));
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

    fn scope(id: &TaskId) -> String {
        format!("moment-{}", task_moment_id(id))
    }

    pub fn run() {
        let out = std::env::var_os("RAMFLUX_SHOTS_DIR").map_or_else(
            || PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("task-shots"),
            PathBuf::from,
        );
        std::fs::create_dir_all(&out).expect("output directory");
        let (fake, state, socket, tasks) = sdk_fake::start_with_tasks("f13b-shots");
        {
            let mut state = state.lock().expect("state");
            state.names.insert(BOT.to_owned(), "Orbit".to_owned());
            state.names.insert("principal-ana".to_owned(), "Ana".to_owned());
        }
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

        // A running task.
        let me = "principal-me";
        tasks.start_as(me, GROUP, "t-notes", BOT, "Draft the release notes for 2.4").unwrap();
        let steps = vec![
            task_step("collect", "Collect the merged changes", TaskStatus::Done),
            task_step("draft", "Write the draft", TaskStatus::Running),
            task_step("post", "Post it for review", TaskStatus::Proposed),
        ];
        tasks.update(GROUP, "t-notes", TaskStatus::Running, steps, None).unwrap();
        let notes = task_key(GROUP, "t-notes");
        run.until("running", |r| r.task(&notes).is_some_and(|t| t.steps.len() == 3));
        run.shot("f13b_01_task_running");

        // The assignee's profile says it is an unattended AI, signed by its
        // root: the task card carries the kind mark (no verified-account
        // check, and a tooltip on what it proves).
        tasks.set_kind(BOT, "headless_ai");
        tasks.verify_kind(BOT);
        tasks.start_as(me, GROUP, "t-kind", BOT, "Summarise the open questions").unwrap();
        let steps = vec![task_step("read", "Read the thread", TaskStatus::Running)];
        tasks.update(GROUP, "t-kind", TaskStatus::Running, steps, None).unwrap();
        let kind = task_key(GROUP, "t-kind");
        run.until("the kind", |r| {
            r.task(&kind).is_some_and(|t| {
                t.actors.assignee.as_ref().is_some_and(|a| a.kind_verified && a.kind.is_some())
            })
        });
        run.shot("f23_05_task_kind_marks");

        // An assignee that used up what it may send, then cancelled.
        tasks.start_as(me, GROUP, "t-index", BOT, "Index the release archive").unwrap();
        let steps = vec![task_step("scan", "Scan the archive", TaskStatus::Running)];
        tasks.update(GROUP, "t-index", TaskStatus::Running, steps, None).unwrap();
        tasks.stall(GROUP, "t-index");
        let index = task_key(GROUP, "t-index");
        run.until("stalled", |r| r.task(&index).is_some_and(|t| t.flags.stalled));
        run.shot("f13b_03_task_stalled");
        run.click(scope(&index), "task-cancel");
        run.until("cancelled", |r| r.task(&index).is_some_and(|t| t.state == TaskState::Cancelled));
        run.shot("f13b_04_task_cancelled");

        // A step that needs the user's approval, decided in place.
        tasks.start_as(me, GROUP, "t-deploy", BOT, "Deploy build 2.4.0-rc1 to staging").unwrap();
        state.lock().expect("state").approvals.push(sdk_fake::approval("appr-deploy", "high"));
        fake.publish("acct-main", "mcp.approval.request", json!({ "approval_id": "appr-deploy" }));
        run.until("the approval", |r| {
            r.cx.update(|cx| {
                r.shell
                    .read(cx)
                    .store()
                    .read(cx)
                    .store()
                    .grant(&GrantId::new("appr-deploy"))
                    .is_some()
            })
        });
        let steps = vec![
            task_step("build", "Build 2.4.0-rc1", TaskStatus::Done),
            approval_step("deploy", "Deploy to staging", "grant_appr-deploy"),
        ];
        tasks.update(GROUP, "t-deploy", TaskStatus::NeedsMe, steps, None).unwrap();
        let deploy = task_key(GROUP, "t-deploy");
        run.until("needs me", |r| r.task(&deploy).is_some_and(|t| t.state == TaskState::NeedsMe));
        run.shot("f13b_02_task_needs_me_approval");

        // Approved with the hold, confirmed by the assignee, then done.
        run.click(scope(&deploy), "step-approve");
        let center = run.window(|window, cx| {
            let center = window.find("hold-confirm").bounds().center();
            window.pointer_down(center, gpui_kit::MouseButton::Left, cx);
            center
        });
        for _ in 0..40 {
            run.tick();
        }
        run.window(move |window, cx| window.pointer_up(center, gpui_kit::MouseButton::Left, cx));
        run.until("the decision on its way", |r| {
            r.task(&deploy).is_some_and(|t| t.answer.is_some())
        });
        // Its delivery to the assignee failed: not delivered, with Retry.
        tasks.fail_delivery(GROUP, "t-deploy", "deploy", "appr-deploy");
        run.until("not delivered", |r| {
            r.task(&deploy).is_some_and(|t| {
                matches!(
                    t.answer,
                    Some(ramflux_desktop_core::TaskAnswer::DecisionUndelivered { .. })
                )
            })
        });
        run.shot("f13b_06_task_decision_not_delivered");
        run.click(scope(&deploy), "task-retry-decision");
        run.until("on its way again", |r| {
            r.task(&deploy).is_some_and(|t| {
                matches!(t.answer, Some(ramflux_desktop_core::TaskAnswer::DecisionPending { .. }))
            })
        });
        tasks.confirm_decisions();
        run.until("running again", |r| {
            r.task(&deploy).is_some_and(|t| t.state == TaskState::Running)
        });
        let steps = vec![
            task_step("build", "Build 2.4.0-rc1", TaskStatus::Done),
            task_step("deploy", "Deploy to staging", TaskStatus::Done),
        ];
        tasks.update(GROUP, "t-deploy", TaskStatus::Done, steps, None).unwrap();
        run.until("done", |r| r.task(&deploy).is_some_and(|t| t.state == TaskState::Done));
        run.shot("f13b_05_task_done");

        // DESK-F17: what members did to tasks are lines, and a task whose
        // step was denied says who declined it (no empty summary).
        tasks.announce_messages(true);
        tasks
            .propose(GROUP, "t-plan", "Deploy build 2.4.0-rc1 to production", &[("prod", "Deploy")])
            .unwrap();
        let plan = task_key(GROUP, "t-plan");
        run.until("the proposal", |r| r.task(&plan).is_some());
        run.click(scope(&plan), "task-start");
        run.until("started", |r| r.task(&plan).is_some_and(|t| t.state == TaskState::Running));
        state.lock().expect("state").approvals.push(sdk_fake::approval("appr-prod", "low"));
        fake.publish("acct-main", "mcp.approval.request", json!({ "approval_id": "appr-prod" }));
        run.until("the production approval", |r| {
            r.cx.update(|cx| {
                r.shell
                    .read(cx)
                    .store()
                    .read(cx)
                    .store()
                    .grant(&GrantId::new("appr-prod"))
                    .is_some()
            })
        });
        let steps = vec![approval_step("prod", "Deploy to production", "grant_appr-prod")];
        tasks.update(GROUP, "t-plan", TaskStatus::NeedsMe, steps, None).unwrap();
        run.until("asks", |r| r.task(&plan).is_some_and(|t| t.state == TaskState::NeedsMe));
        run.click(scope(&plan), "step-deny");
        run.until("denied", |r| r.task(&plan).is_some_and(|t| t.answer.is_some()));
        // The denial ends the task as failed, without a result.
        tasks.confirm_decisions();
        run.until("failed", |r| r.task(&plan).is_some_and(|t| t.state == TaskState::Failed));
        // The conversation's next read brings the event messages.
        let workspace = run.cx.update(|cx| run.shell.read(cx).workspace().clone());
        run.window(move |window, cx| {
            workspace.update(cx, |w, cx| {
                w.select_context(None, window, cx);
                w.select_context(Some(ContextId::new(GROUP)), window, cx);
            });
        });
        run.until("the lines", |r| {
            r.window(|window, _| window.within(scope(&plan)).try_find("task-ending").is_some())
                && r.window(|window, _| window.find_all("task-event").len() >= 2)
        });
        run.shot("f17_01_task_lines_and_declined");
        fake.shutdown();
    }
}
