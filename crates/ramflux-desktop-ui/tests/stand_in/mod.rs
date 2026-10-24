// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The stand-in `rf` for tests that start a local daemon: the test binary
//! itself.
//!
//! The launcher starts the program with the daemon's arguments
//! (`--socket <socket> daemon start --data-root <dir>`); the test binary's
//! `main` calls [`run_if_started`] first, which acts as the daemon and
//! never returns. The binary is already running when a test starts it
//! again, so no program new to the system runs inside a test's wait: on
//! macOS the first exec of a newly written executable (a shell script made
//! for each run, as before) is held while the system checks it, from about
//! 0.3 s to over 20 s under load.
//!
//! What the stand-in does is written next to the socket by [`program`],
//! since every start gets the same arguments shape.

#![allow(dead_code)]

use std::ffi::OsString;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The file next to the socket that says what the stand-in does.
const BEHAVIOR: &str = "stand-in";

/// What the stand-in does once it has recorded its arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Behavior<'a> {
    /// Keeps running until it is stopped.
    Wait,
    /// Writes `message` to its error output and exits with `code`.
    Exit { code: i32, message: &'a str },
}

/// Makes the stand-in started for a socket in `dir` behave as `behavior`,
/// and returns the program to start: `dir/rf`, a symbolic link to this
/// test binary (on Unix; the binary itself elsewhere). A link is not a new
/// file, so starting it is not checked again.
pub fn program(dir: &Path, behavior: Behavior<'_>) -> PathBuf {
    let text = match behavior {
        Behavior::Wait => "wait".to_owned(),
        Behavior::Exit { code, message } => format!("exit {code}\n{message}"),
    };
    std::fs::write(dir.join(BEHAVIOR), text).expect("stand-in behavior");
    let binary = std::env::current_exe().expect("this test binary");
    #[cfg(unix)]
    {
        let link = dir.join("rf");
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(&binary, &link).expect("link to the test binary");
        link
    }
    #[cfg(not(unix))]
    binary
}

/// When this process was started as the daemon, acts as the stand-in and
/// exits; otherwise returns.
pub fn run_if_started() {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    if args.first().is_some_and(|a| a == "--socket") && args.len() > 1 {
        stand_in(&args);
    }
}

/// Records the arguments next to the socket (`args`, renamed into place
/// once complete), then does what the behavior file says.
fn stand_in(args: &[OsString]) -> ! {
    let socket = PathBuf::from(&args[1]);
    let dir = socket.parent().expect("socket dir");
    let line: Vec<String> = args.iter().map(|a| a.to_string_lossy().into_owned()).collect();
    let partial = dir.join("args.partial");
    std::fs::write(&partial, format!("{}\n", line.join(" "))).expect("args");
    std::fs::rename(&partial, dir.join("args")).expect("args in place");
    let behavior = std::fs::read_to_string(dir.join(BEHAVIOR)).unwrap_or_default();
    let mut lines = behavior.lines();
    let code = lines.next().and_then(|l| l.strip_prefix("exit ")).and_then(|c| c.parse().ok());
    match code {
        Some(code) => {
            let message: Vec<&str> = lines.collect();
            let mut stderr = std::io::stderr();
            let _ = writeln!(stderr, "{}", message.join("\n"));
            std::process::exit(code);
        }
        None => {
            println!("started");
            // Stopped by the test (terminate signal), or gone after this.
            std::thread::sleep(Duration::from_secs(30));
            std::process::exit(0);
        }
    }
}
