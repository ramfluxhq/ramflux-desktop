#!/usr/bin/env python3
"""Execute every bus-alignment regression introduced so far, with positive counts."""

import re
import subprocess

# Extend these lists in the PR that introduces each case. Future cases must
# not block earlier preparatory PRs; existing cases must remain required.
SDK_TESTS = [
    "local::launch::tests::old_settings_file_loads",
    "bridge::scope::idle_tests::no_periodic_requests_when_idle",
    "bridge::scope::idle_tests::failed_stream_only_reopens_subscription_without_data_polls",
    "bridge::scope::idle_tests::daemon_scope_refreshes_on_actions_not_timers",

    "bridge::scope::networking_tests::authority_restored_event_clears_blocked_and_fences_delayed_status",
    "bridge::scope::networking_tests::custody_pending_and_authority_blocked_override_live_session",
    "bridge::scope::networking_tests::daemon_list_and_lifecycle_merge_preserves_account_session",
    "bridge::scope::networking_tests::push_gap_reopen_recovers_networking_snapshot",
    "bridge::scope::networking_tests::sessionless_terminal_reason_survives_delayed_status",

    "bus::tests::a_retry_the_daemon_cannot_replay_asks_the_user_to_check",
    "bridge::scope::review_retry_tests::recovered_reads_resume_timer_refresh_and_reset_failure_episode",
    "bridge::scope::review_retry_tests::backup_permanent_refusal_sets_capability_and_finishes_listing",
    "bridge::scope::review_retry_tests::in_flight_refresh_reads_committed_state_once_after_all_completion_paths",
    "bridge::scope::networking_tests::networking_changed_event_updates_header_without_status_read",
    "bridge::scope::networking_tests::missed_networking_event_is_replayed_from_cursor",

    "bridge::scope::networking_tests::admitted_disconnected_account_attaches_as_pending",
    "bridge::scope::networking_tests::expired_cursor_recovers_networking_snapshot",
    "bridge::scope::networking_tests::delayed_status_cannot_overwrite_newer_networking_event",

    "bridge::scope::requests_retry_tests::unlocked_status_is_delivered_before_slow_snapshot_lists",
    "bridge::scope::requests_retry_tests::read_timeout_keeps_rows_and_retries_once",
    "bridge::scope::requests_retry_tests::read_retry_cancels_on_lock_or_detach_and_coalesces",
    "bus::deadline::tests::unlock_budget_and_weights",
    "bus::tests::reconciliation_refusal_is_matched_by_code",
    "bridge::s5_lifecycle_tests::reconcile_retry_sends_a_fresh_key",
    "bridge::s5_lifecycle_tests::lifecycle_answer_lost_resends_once_with_same_key",
    "local::daemon_process::tests::daemon_spawn_sets_default_rust_log_unless_inherited",
]
UI_TESTS = [
    "authority_restored_event_returns_blocked_header_to_live",
    "unlock_opens_account_on_networking_pending",
    "header_shows_connecting_until_live_or_typed_reason",
]
GUARD = "a_lost_answer_is_resent_under_the_same_key_and_applied_once"


def run(command):
    result = subprocess.run(command, timeout=300, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, check=False)
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
    required(["cargo", "test", "--locked", "-p", "ramflux-desktop-sdk", "--test", "custody"],
        "a_pending_account_follows_networking_events_without_status_polling")
    required(["cargo", "test", "--locked", "-p", "ramflux-desktop-ui", "--test", "demo_rehearsal_fake"],
        "every_step_passes_on_a_node_that_behaves")
    for features in ([], ["--features", "bus-trace"]):
        for name in SDK_TESTS:
            required(["cargo", "test", "--locked", "-p", "ramflux-desktop-sdk", "--lib", *features], name)
    required(["cargo", "test", "--locked", "-p", "ramflux-desktop-sdk", "--test", "requests"], GUARD)
    required(
        ["cargo", "test", "--locked", "-p", "ramflux-desktop-sdk", "--test", "attach_flow"],
        "node_profiles_resolve_the_relay_after_early_account_status",
    )
    for name in (
        "the_restore_result_stays_in_view_when_the_daemon_reopens_the_account",
        "a_daemon_that_goes_away_during_the_hold_releases_it",
    ):
        required(["cargo", "test", "--locked", "-p", "ramflux-desktop-ui", "--test", "backup_ui"], name)
    required(
        ["cargo", "test", "--locked", "-p", "ramflux-desktop-ui", "--test", "ceremony_ui"],
        "the_asked_words_complete_from_a_prefix_and_confirming_creates_the_account",
    )
    target = ["cargo", "test", "--locked", "-p", "ramflux-desktop-ui", "--test", "s5_bus_align"]
    # Compile the normal libtest target even before its first owned case lands.
    run([*target, "--", "--list"])
    for name in UI_TESTS:
        required(target, name)
    print("OK: all introduced bus alignment tests executed with positive counts")


if __name__ == "__main__":
    main()
