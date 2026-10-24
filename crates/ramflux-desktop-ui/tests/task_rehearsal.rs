// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Demo ③ rehearsed against a real node: Alice on two offscreen desktops
//! (two devices of one principal, each on its own `rf daemon`) and Orbit,
//! an AI participant on a third daemon, played over its local bus (see
//! `demo3`). Every step asserts the stores and saves light and dark
//! screenshots; the step table is printed and written next to them as
//! `steps.md`.
//!
//! Not part of `cargo test` and never run in CI; run explicitly:
//!
//! ```sh
//! RAMFLUX_DEMO_REHEARSAL=1 \
//! RAMFLUX_DEMO_NODE_URL=https://node.example.org \
//! RAMFLUX_DEMO_NODE_KEY=<the node public key, or its fingerprint> \
//! RAMFLUX_RF_BIN=/path/to/bin/rf \
//! RAMFLUX_DEMO_SHOTS_DIR=/some/dir \
//!   cargo test -p ramflux-desktop-ui --test task_rehearsal
//! ```
//!
//! Accounts created on the node are demo data and stay there.

mod demo3;
mod rehearsal;

fn main() {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    live::run();
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    println!("task rehearsal: skipped; GPUI has no headless renderer on this platform");
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod live {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::Duration;

    use gpui_kit::HeadlessAppContext;
    use gpui_kit::assets::Assets;
    use ramflux_desktop_sdk::SdkConfig;
    use ramflux_desktop_test::live_daemon::LiveDaemon;
    use ramflux_desktop_ui::shell::{ShellOptions, SourceChoice};

    use crate::demo3::{self, Demo3};
    use crate::rehearsal::config::{self, DemoConfig};

    /// A desktop on the daemon at `socket`: the event stream only, polling
    /// off.
    fn options(socket: &Path) -> ShellOptions {
        let config = SdkConfig {
            poll_fallback: false,
            reconnect_backoff: Duration::from_millis(300),
            ..SdkConfig::new(socket.to_path_buf())
        };
        ShellOptions { source: SourceChoice::Rfd(config), ..ShellOptions::for_scenario("", true) }
    }

    pub fn run() {
        let default_shots = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("task-rehearsal");
        let config = match DemoConfig::from_env(default_shots) {
            Ok(Some(config)) => config,
            Ok(None) => {
                println!(
                    "task rehearsal: skipped; set {}=1, {} and {} to run it against a node",
                    config::ENABLE_ENV,
                    config::NODE_URL_ENV,
                    config::NODE_KEY_ENV
                );
                return;
            }
            Err(missing) => panic!("task rehearsal: {missing}"),
        };
        let dir = config.shots.clone().expect("a live run takes screenshots");
        std::fs::create_dir_all(&dir).expect("screenshot folder");
        let daemons = ["d3-a1", "d3-a2", "d3-ai"]
            .map(|tag| LiveDaemon::start(tag).unwrap_or_else(|e| panic!("daemon {tag}: {e}")));
        let mut cx = HeadlessAppContext::with_platform(
            gpui_kit::platform::current_platform(true).text_system(),
            Arc::new(Assets),
            gpui_kit::platform::current_headless_renderer,
        );
        cx.update(|cx| {
            gpui_kit::init(cx);
            ramflux_desktop_ui::init(cx);
        });
        println!("task rehearsal {} against {}", config.run_id, config.node_url);
        let mut demo = Demo3::new(&mut cx, config, Box::new(options), daemons);
        for dir in demo3::dirs(&demo) {
            println!("daemon dir {}", dir.display());
        }
        let steps = demo3::run(&mut demo);
        let table = steps.markdown();
        println!("\n{table}");
        std::fs::write(dir.join("steps.md"), &table).expect("write the step table");
        println!("screenshots and steps.md in {}", dir.display());
        drop(demo);
        drop(cx);
        assert!(steps.all_passed(), "the rehearsal did not pass every step");
    }
}
