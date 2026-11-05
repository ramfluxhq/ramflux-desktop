#!/usr/bin/env python3
"""Execute every bus-alignment regression introduced so far, with positive counts."""

import re
import subprocess

# Extend these lists in the PR that introduces each case. Future cases must
# not block earlier preparatory PRs; existing cases must remain required.
SDK_TESTS = [
    "bus::tests::a_retry_the_daemon_cannot_replay_asks_the_user_to_check",
    "bus::tests::reconciliation_refusal_is_matched_by_code",
    "bridge::s5_lifecycle_tests::reconcile_retry_sends_a_fresh_key",
    "bridge::s5_lifecycle_tests::lifecycle_answer_lost_resends_once_with_same_key",
    "local::daemon_process::tests::daemon_spawn_sets_default_rust_log_unless_inherited",
]
UI_TESTS = []  # First cases belong to the networking/header PR.
GUARD = "a_lost_answer_is_resent_under_the_same_key_and_applied_once"


def run(command):
    result = subprocess.run(command, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, check=False)
    print(result.stdout, end="", flush=True)
    if result.returncode:
        raise SystemExit(result.returncode)
    return result.stdout


def required(target, name):
    listed = run([*target, name, "--", "--exact", "--list"])
    if listed.splitlines().count(f"{name}: test") != 1:
        raise SystemExit(f"required test missing or duplicated: {name}")
    output = run([*target, name, "--", "--exact"])
    if not re.search(r"^test result: ok\. 1 passed; 0 failed;", output, re.MULTILINE):
        raise SystemExit(f"required test did not execute and pass once: {name}")


def main():
    for features in ([], ["--features", "bus-trace"]):
        for name in SDK_TESTS:
            required(["cargo", "test", "--locked", "-p", "ramflux-desktop-sdk", "--lib", *features], name)
    required(["cargo", "test", "--locked", "-p", "ramflux-desktop-sdk", "--test", "requests"], GUARD)
    target = ["cargo", "test", "--locked", "-p", "ramflux-desktop-ui", "--test", "s5_bus_align"]
    # Compile the normal libtest target even before its first owned case lands.
    run([*target, "--", "--list"])
    for name in UI_TESTS:
        required(target, name)
    print("OK: all introduced bus alignment tests executed with positive counts")


if __name__ == "__main__":
    main()
