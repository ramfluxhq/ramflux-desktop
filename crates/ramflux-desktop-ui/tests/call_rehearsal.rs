// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Voice calls rehearsed against a real node with the CALL-01 voice relay:
//! two offscreen desktops, each on its own `rf daemon` from the pinned
//! product revision, set up as in the demo (node, accounts, contacts, a
//! direct conversation), then call each other (see `rehearsal::flow::call`):
//! answered with audio both ways through the relay, declined, unanswered.
//! No audio devices: each desktop's call audio is the production media
//! engine with a test tone for a microphone and a meter for a speaker.
//! Every step asserts the stores and saves light and dark screenshots; the
//! step table is printed and written next to them as `steps.md`.
//!
//! Not part of `cargo test` and never run in CI; run explicitly:
//!
//! ```sh
//! RAMFLUX_DEMO_REHEARSAL=1 \
//! RAMFLUX_DEMO_NODE_URL=https://node.example.org \
//! RAMFLUX_DEMO_NODE_KEY=<the node public key, or its fingerprint> \
//! RAMFLUX_RF_BIN=/path/to/bin/rf \
//! RAMFLUX_DEMO_SHOTS_DIR=/some/dir \
//!   cargo test -p ramflux-desktop-ui --test call_rehearsal
//! ```

mod rehearsal;

fn main() {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    live::run();
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    println!("call rehearsal: skipped; GPUI has no headless renderer on this platform");
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
    use crate::rehearsal::flow::call::ToneAudio;
    use crate::rehearsal::flow::{self, Demo, Side};

    /// A desktop on the daemon at `socket`: the event stream only, polling
    /// off; calls on (the daemon answers `call.list`).
    fn options(socket: &Path) -> ShellOptions {
        let config = SdkConfig {
            poll_fallback: false,
            reconnect_backoff: Duration::from_millis(300),
            ..SdkConfig::new(socket.to_path_buf())
        };
        ShellOptions { source: SourceChoice::Rfd(config), ..ShellOptions::for_scenario("", true) }
    }

    pub fn run() {
        let default_shots = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("call-rehearsal");
        let config = match DemoConfig::from_env(default_shots) {
            Ok(Some(config)) => config,
            Ok(None) => {
                println!(
                    "call rehearsal: skipped; set {}=1, {} and {} to run it against a node",
                    config::ENABLE_ENV,
                    config::NODE_URL_ENV,
                    config::NODE_KEY_ENV
                );
                return;
            }
            Err(missing) => panic!("call rehearsal: {missing}"),
        };
        let dir = config.shots.clone().expect("a live run takes screenshots");
        std::fs::create_dir_all(&dir).expect("screenshot folder");
        let work = PathBuf::from(format!("/tmp/rf-f16-{}", std::process::id()));
        std::fs::create_dir_all(&work).expect("work folder");
        let alice = LiveDaemon::start("call-a").unwrap_or_else(|e| panic!("daemon A: {e}"));
        let bob = LiveDaemon::start("call-b").unwrap_or_else(|e| panic!("daemon B: {e}"));
        for daemon in [&alice, &bob] {
            println!("daemon dir {}", daemon.dir().display());
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
        println!("call rehearsal {} against {}", config.run_id, config.node_url);
        let run = config.run_id.clone();
        let audio = [ToneAudio::new(440.0, 660.0), ToneAudio::new(660.0, 440.0)];
        let mut demo = Demo::new(
            &mut cx,
            config,
            Box::new(options),
            Side::new("a", "Alice", alice).labelled(format!("call-alice-{run}")),
            Side::new("b", "Bob", bob).labelled(format!("call-bob-{run}")),
            work.clone(),
        );
        let steps = flow::run_calls(&mut demo, &audio);
        let table = steps.markdown();
        println!("\n{table}");
        std::fs::write(dir.join("steps.md"), &table).expect("write the step table");
        println!("screenshots and steps.md in {}", dir.display());
        drop(demo);
        drop(cx);
        let _ = std::fs::remove_dir_all(&work);
        assert!(steps.all_passed(), "the rehearsal did not pass every step");
    }
}
