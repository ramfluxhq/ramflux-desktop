// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Request-list retry scheduling across stream and polling transitions.

use super::*;
use crate::bus::stream::Streamed;
use ramflux_desktop_core::{AccountId, SyncMode};
use ramflux_desktop_test::fake_bus::{FakeBus, Reply, socket_path};
use serde_json::json;

fn task() -> (Task, Inbox, std_mpsc::Receiver<Delivery>) {
    let mut config = SdkConfig::new("retry-test.sock");
    config.poll_fast = Duration::from_millis(80);
    config.poll_slow = Duration::from_millis(300);
    let (outbox, delivered) = std_mpsc::channel();
    let (phrases, _) = std_mpsc::channel();
    let (mut task, inbox) = build(
        config,
        Scope::Account(AccountId::new("acct-main")),
        outbox,
        Ceremonies { phrases, audit: Arc::new(super::super::PhraseAudit::default()) },
    );
    task.link = Link::Live;
    task.sync.mode = SyncMode::Stream;
    (task, inbox, delivered)
}

#[test]
fn persistent_request_failures_back_off_to_slow_and_success_resets_the_wait() {
    let (mut task, _inbox, _delivered) = task();
    for milliseconds in [80, 160, 300, 300] {
        let before = Instant::now();
        task.on_read(&Fetch::ContactRequests, Err(BusError::Timeout));
        assert_eq!(task.requests_retry_wait, Duration::from_millis(milliseconds));
        let due = task.requests_retry_at.expect("retry armed");
        assert!(due >= before + Duration::from_millis(milliseconds));
        assert_eq!(task.next_wake(), due);
    }
    task.on_read(&Fetch::ContactRequests, Ok(json!({ "requests": [] })));
    assert!(task.requests_retry_at.is_none());
    assert_eq!(task.requests_retry_wait, Duration::ZERO);
    task.on_read(&Fetch::ContactRequests, Err(BusError::Timeout));
    assert_eq!(task.requests_retry_wait, task.config.poll_fast);
}

#[test]
fn unknown_method_connection_loss_and_lock_clear_request_retry_state() {
    for clear in 0..3 {
        let (mut task, _inbox, _delivered) = task();
        task.on_read(&Fetch::ContactRequests, Err(BusError::Timeout));
        assert!(task.requests_retry_at.is_some());
        match clear {
            0 => {
                task.on_read(
                    &Fetch::ContactRequests,
                    Err(BusError::daemon("ValidationFailed", "unsupported local bus method")),
                );
            }
            1 => {
                task.on_read(
                    &Fetch::ContactRequests,
                    Err(BusError::Transport("connection closed".into())),
                );
                assert_eq!(task.link, Link::Lost);
            }
            _ => task.locked(),
        }
        assert!(task.requests_retry_at.is_none());
        assert_eq!(task.requests_retry_wait, Duration::ZERO);
    }
}

#[tokio::test]
async fn a_polling_failure_is_retried_after_stream_reopens_with_a_cursor_then_stops() {
    let path = socket_path("requests-retry-cursor");
    let fake = FakeBus::start(&path, |_, _, _| Reply::Ok(json!({ "requests": [] })));
    let (mut task, mut inbox, _delivered) = task();
    task.client = BusClient::new(path, task.config.connect_timeout, task.config.deadlines);
    task.sync.mode = SyncMode::Polling;
    task.sync.cursor = Some(42);
    task.polls_in_flight = 1;
    task.subscribed = true;
    task.on_read(&Fetch::ContactRequests, Err(BusError::Timeout));
    assert!(task.requests_retry_at.is_some(), "a failed polling read also arms recovery");
    task.requests_retry_at = Some(Instant::now());
    assert_ne!(Some(task.next_wake()), task.requests_retry_at, "polling ignores the retry wake");
    task.tick();
    assert!(task.requests_retry_at.is_some(), "polling retains the retry for stream recovery");

    // No cursor gap: stream_opened does not reload the snapshot. The
    // pending retry must survive this exact polling-to-stream transition.
    task.on_streamed(Streamed::Opened { head_seq: 42, floor_seq: 0 });
    assert_eq!(task.sync.mode, SyncMode::Stream);
    assert!(fake.seen().is_empty(), "opening with a cursor did not reload");
    task.tick();
    let answer = tokio::time::timeout(Duration::from_secs(3), inbox.internal.recv())
        .await
        .expect("retry answered")
        .expect("internal channel");
    assert!(matches!(&answer, Internal::Fetched((Fetch::ContactRequests, _, Ok(_)))));
    task.internal(answer);
    assert_eq!(fake.seen().len(), 1);
    assert_eq!(fake.seen()[0].method, "contact.requests.list");
    assert!(task.requests_retry_at.is_none(), "success cancels retry-originated reads");
    assert_eq!(task.requests_retry_wait, Duration::ZERO);
    task.tick();
    assert!(inbox.internal.try_recv().is_err(), "no retry work was scheduled after success");
    fake.shutdown();
}
