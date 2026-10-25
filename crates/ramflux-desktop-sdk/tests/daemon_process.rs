// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! A daemon the desktop starts gets the socket and data root, writes to its
//! log, and is stopped and reaped.
//!
//! The stand-in daemon is this test binary itself: started with the
//! daemon's arguments (`--socket <socket> daemon start ...`), `main` acts as
//! the daemon instead of running the tests. The binary is already running
//! when the test starts, so starting it again does not run a program that
//! is new to the system. A stand-in written for each run (a shell script,
//! as before) is a new file every time, and on macOS the first exec of a
//! new executable is held while the system checks it (0.3 s to over 20 s
//! measured on a developer Mac), which ran past the test's wait. That is
//! also why this file has its own `main`: libtest would reject the daemon's
//! arguments.

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use ramflux_desktop_sdk::local::{DaemonCommand, DaemonProcess};

/// The stand-in daemon: records its arguments next to the socket, says it
/// started, then waits to be stopped. The arguments are written to a
/// temporary file and renamed into place, so the file exists only once it
/// is complete.
fn stand_in_daemon(args: &[OsString]) {
    let socket = PathBuf::from(&args[1]);
    let dir = socket.parent().expect("socket dir");
    let lines: Vec<String> = args.iter().map(|a| a.to_string_lossy().into_owned()).collect();
    let partial = dir.join("args.partial");
    std::fs::write(&partial, lines.join("\n")).expect("args");
    std::fs::rename(&partial, dir.join("args")).expect("args in place");
    println!("started");
    std::thread::sleep(Duration::from_secs(30));
}

fn dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rf-proc-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("dir");
    dir
}

fn pid_exists(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// A `ps` column of `pid`.
fn ps(pid: u32, column: &str) -> Option<u32> {
    let out = std::process::Command::new("ps")
        .args(["-o", &format!("{column}="), "-p", &pid.to_string()])
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

fn a_started_daemon_gets_the_socket_and_data_root_and_stops() {
    let dir = dir("run");
    let command = DaemonCommand {
        binary: std::env::current_exe().expect("this binary"),
        socket: dir.join("rfd.sock"),
        data_dir: dir.join("data"),
        log_file: Some(dir.join("logs/rfd.log")),
    };
    let mut process = DaemonProcess::spawn(&command).expect("spawn");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !dir.join("args").exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    let args = std::fs::read_to_string(dir.join("args")).expect("args");
    let sent: Vec<String> = args.lines().map(str::to_owned).collect();
    assert_eq!(
        sent.join(" "),
        format!(
            "--socket {} daemon start --data-root {}",
            dir.join("rfd.sock").display(),
            dir.join("data").display()
        )
    );
    assert_eq!(process.exit_code(), None, "still running");
    let pid = process.pid();
    // The daemon stays the app's own child, in the app's process group:
    // macOS attributes its local network use to the app that spawned it
    // (the responsible code, TN3179), so the app's Local Network
    // permission covers it. A daemon that detached itself (a new session,
    // or a launchd job) would need a permission of its own (DESK-F19).
    let me = std::process::id();
    assert_eq!(ps(pid, "ppid"), Some(me), "the daemon is this process's child");
    assert_eq!(ps(pid, "pgid"), ps(me, "pgid"), "in this process's group");
    process.stop(Duration::from_secs(3));
    assert!(process.wait_exit(Duration::ZERO), "exited");
    assert!(!pid_exists(pid), "reaped");
    let log = std::fs::read_to_string(dir.join("logs/rfd.log")).expect("log");
    assert!(log.contains("started"), "output goes to the log");
    let _ = std::fs::remove_dir_all(dir);
}

type Test = (&'static str, fn());

const TESTS: &[Test] = &[(
    "a_started_daemon_gets_the_socket_and_data_root_and_stops",
    a_started_daemon_gets_the_socket_and_data_root_and_stops,
)];

/// Runs the tests whose name contains every non-flag argument, and reports
/// in libtest's format.
fn run_tests(args: &[OsString]) -> bool {
    let filters: Vec<String> = args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .filter(|a| !a.starts_with('-'))
        .collect();
    let chosen: Vec<&Test> = TESTS
        .iter()
        .filter(|(name, _)| filters.iter().all(|f| name.contains(f.as_str())))
        .collect();
    if args.iter().any(|a| a == "--list") {
        for (name, _) in &chosen {
            println!("{name}: test");
        }
        return true;
    }
    println!("\nrunning {} tests", chosen.len());
    let mut failed = Vec::new();
    for (name, test) in &chosen {
        let ok = std::panic::catch_unwind(test).is_ok();
        println!("test {name} ... {}", if ok { "ok" } else { "FAILED" });
        if !ok {
            failed.push(*name);
        }
    }
    if !failed.is_empty() {
        println!("\nfailures:");
        for name in &failed {
            println!("    {name}");
        }
    }
    println!(
        "\ntest result: {}. {} passed; {} failed; 0 ignored; 0 measured; {} filtered out\n",
        if failed.is_empty() { "ok" } else { "FAILED" },
        chosen.len() - failed.len(),
        failed.len(),
        TESTS.len() - chosen.len()
    );
    failed.is_empty()
}

fn main() {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    if args.first().is_some_and(|a| a == "--socket") {
        stand_in_daemon(&args);
        return;
    }
    // The test stops the daemon with `kill`, so it runs on unix only.
    if cfg!(unix) && !run_tests(&args) {
        std::process::exit(101);
    }
}
