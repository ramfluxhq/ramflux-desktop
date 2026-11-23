// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The tier ② demo rehearsed against a real node: two offscreen desktops,
//! each on its own `rf daemon` from the pinned product revision, walk
//! through node, accounts, contacts, messages, an attachment, lock and a
//! daemon restart (see `rehearsal::flow`). Every step asserts the store and
//! saves light and dark screenshots; the step table is printed and written
//! next to them as `steps.md`.
//!
//! Not part of `cargo test` and never run in CI; run explicitly:
//!
//! ```sh
//! RAMFLUX_DEMO_REHEARSAL=1 \
//! RAMFLUX_DEMO_NODE_URL=https://node.example.org \
//! RAMFLUX_DEMO_NODE_KEY=<the node public key, or its fingerprint> \
//! RAMFLUX_RF_BIN=/path/to/bin/rf \
//! RAMFLUX_DEMO_SHOTS_DIR=/some/dir \
//! RAMFLUX_DEMO_PEERS=/path/to/peers.json \
//!   cargo test -p ramflux-desktop-ui --test demo_rehearsal
//! ```
//!
//! With `RAMFLUX_DEMO_PEERS` (the node's list of command-line test peers)
//! the run continues with Alice's desktop and those peers (steps 20 to 24,
//! see `rehearsal::flow::peers`).
//!
//! Accounts created on the node are demo data and stay there; nothing on
//! the node is removed. The daemons and their folders are removed at the
//! end, also when a step fails.

mod rehearsal;

fn main() {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    live::run();
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    println!("demo rehearsal: skipped; GPUI has no headless renderer on this platform");
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod live {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::Duration;

    use gpui_kit::HeadlessAppContext;
    use ramflux_desktop_sdk::SdkConfig;
    use ramflux_desktop_test::live_daemon::LiveDaemon;
    use ramflux_desktop_ui::AppAssets as Assets;
    use ramflux_desktop_ui::shell::{ShellOptions, SourceChoice};

    use crate::rehearsal::config::{self, DemoConfig};
    use crate::rehearsal::flow::{self, Demo, Side};
    use crate::rehearsal::tap::Tap;

    /// The desktop's options on a daemon at `socket`: the event stream
    /// only, polling off, so every message the desktop shows came through
    /// the stream (Diagnostics says "not updated" if the daemon refuses it).
    /// The desktop attaches through the frame tap next to the daemon socket
    /// (`desk.sock`, see `rehearsal::tap`) and keeps its settings, the
    /// stream cursor included, in `desktop-settings.json` beside it, so a
    /// reopened desktop resumes from the cursor its predecessor saved.
    fn options(socket: &Path) -> ShellOptions {
        let dir = socket.parent().unwrap_or(Path::new("/tmp"));
        let config = SdkConfig {
            reconnect_backoff: Duration::from_millis(300),
            settings_path: Some(dir.join(SETTINGS_FILE)),
            ..SdkConfig::new(dir.join(TAP_SOCKET))
        };
        ShellOptions { source: SourceChoice::Rfd(config), ..ShellOptions::for_scenario("", true) }
    }

    /// The desktop's socket: the tap in front of the daemon.
    const TAP_SOCKET: &str = "desk.sock";
    /// The tap's frame log.
    const TAP_LOG: &str = "tap.jsonl";
    /// The desktop settings file (stream cursors).
    const SETTINGS_FILE: &str = "desktop-settings.json";

    /// A scratch folder removed on drop.
    struct Work(PathBuf);

    impl Drop for Work {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    pub fn run() {
        let default_shots = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("demo-rehearsal");
        let config = match DemoConfig::from_env(default_shots) {
            Ok(Some(config)) => config,
            Ok(None) => {
                println!(
                    "demo rehearsal: skipped; set {}=1, {} and {} to run it against a node",
                    config::ENABLE_ENV,
                    config::NODE_URL_ENV,
                    config::NODE_KEY_ENV
                );
                return;
            }
            Err(missing) => panic!("demo rehearsal: {missing}"),
        };
        let dir = config.shots.clone().expect("a live run takes screenshots");
        std::fs::create_dir_all(&dir).expect("screenshot folder");
        let work = Work(PathBuf::from(format!("/tmp/rf-f9-{}", std::process::id())));
        let _ = std::fs::remove_dir_all(&work.0);
        std::fs::create_dir_all(&work.0).expect("work folder");
        let alice = LiveDaemon::start("demo-a").unwrap_or_else(|e| panic!("daemon A: {e}"));
        let bob = LiveDaemon::start("demo-b").unwrap_or_else(|e| panic!("daemon B: {e}"));
        let taps: Vec<Tap> = [&alice, &bob]
            .iter()
            .map(|d| {
                Tap::start(&d.dir().join(TAP_SOCKET), &d.socket(), &d.dir().join(TAP_LOG))
                    .unwrap_or_else(|e| panic!("frame tap: {e}"))
            })
            .collect();
        for daemon in [&alice, &bob] {
            println!("daemon dir {}", daemon.dir().display());
        }
        // A desktop closed after its daemon can write its settings there
        // again: both folders are removed once everything is dropped.
        let daemon_dirs: Vec<PathBuf> =
            [&alice, &bob].iter().map(|d| d.dir().to_path_buf()).collect();
        let mut cx = HeadlessAppContext::with_platform(
            gpui_kit::platform::current_platform(true).text_system(),
            Arc::new(Assets),
            gpui_kit::platform::current_headless_renderer,
        );
        cx.update(|cx| {
            gpui_kit::init(cx);
            ramflux_desktop_ui::init(cx);
        });
        println!("demo rehearsal {} against {}", config.run_id, config.node_url);
        let run = config.run_id.clone();
        let mut demo = Demo::new(
            &mut cx,
            config,
            Box::new(options),
            Side::new("a", "Alice", alice).labelled(format!("rehearsal-alice-{run}")),
            Side::new("b", "Bob", bob).labelled(format!("rehearsal-bob-{run}")),
            work.0.clone(),
        );
        let peers = flow::peers::Peer::from_env().unwrap_or_else(|e| panic!("peers: {e}"));
        if peers.is_none() {
            println!(
                "node peer steps: skipped; set {} to the node's peers.json",
                flow::peers::PEERS_ENV
            );
        }
        let steps = flow::run_live(&mut demo, peers);
        let table = steps.markdown();
        println!("\n{table}");
        std::fs::write(dir.join("steps.md"), &table).expect("write the step table");
        println!("screenshots and steps.md in {}", dir.display());
        drop(demo);
        drop(cx);
        drop(taps);
        drop(work);
        for dir in daemon_dirs {
            let _ = std::fs::remove_dir_all(dir);
        }
        assert!(steps.all_passed(), "the rehearsal did not pass every step");
    }
}
