// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The MLS groups store: the join owner, the timeline merge, gaps, the AI
//! marker merge, the send owner (eviction, dedupe, the rate lock) and the
//! redaction of links.

mod common;
mod mls_samples;

use common::*;
use ramflux_desktop_core::mls_group::*;
use ramflux_desktop_core::*;

const G: &str = "g-1";

fn feed() -> Feed {
    let mut feed = Feed::new();
    let mut summary = mls_samples::summary();
    summary.group_id = G.to_owned();
    summary.read_log_seq = 9;
    feed.ok(
        1,
        DomainEvent::MlsGroupsListed { rows: vec![summary], cursor: None, first_page: true },
    );
    feed
}

fn post(log_seq: u64, id: &str, text: &str) -> Post {
    Post {
        log_seq: Some(log_seq),
        post_id: id.to_owned(),
        body: PostBody::Text(text.to_owned()),
        ..mls_samples::post()
    }
}

fn mine(mut post: Post) -> Post {
    post.author =
        Some(Author { anchor: "anc-me".into(), display: None, ai_flag: false, is_me: true });
    post
}

fn history(feed: &mut Feed, direction: HistoryDirection, posts: Vec<Post>, last: u64, more: bool) {
    feed.ok(
        10,
        DomainEvent::MlsHistoryPage {
            group_id: G.to_owned(),
            direction,
            posts,
            floor_log_seq: Some(1),
            last_log_seq: last,
            more,
        },
    );
}

fn moments(feed: &Feed) -> Vec<String> {
    feed.store
        .context_moments(&mls_group::context_id(G))
        .map(|m| m.id.as_str().to_owned())
        .collect()
}

fn event(feed: &mut Feed, seq: u64, log_seq: Option<u64>, kind: GroupEventKind) {
    feed.ok(20, DomainEvent::MlsGroupEvent { seq, group_id: G.to_owned(), log_seq, kind });
}

fn started(feed: &mut Feed, local: &str) {
    feed.ok(
        30,
        DomainEvent::MlsStarted(MlsStart::Send {
            group_id: G.to_owned(),
            local_id: local.to_owned(),
            text: "hi".to_owned(),
            reply_to: None,
            mentions: Vec::new(),
            retry_of: None,
        }),
    );
}

fn outcome(feed: &mut Feed, local: &str, post: Option<&str>, state: SendState) {
    feed.ok(
        31,
        DomainEvent::MlsSendOutcome {
            group_id: G.to_owned(),
            local_id: local.to_owned(),
            post_id: post.map(str::to_owned),
            state,
        },
    );
}

#[test]
fn a_join_outcome_of_an_older_attempt_is_recorded_but_does_not_replace_a_newer_one() {
    let mut feed = feed();
    feed.ok(2, DomainEvent::MlsStarted(MlsStart::Join { attempt: 1 }));
    feed.ok(
        3,
        DomainEvent::MlsJoinOutcome {
            attempt: 2,
            outcome: JoinOutcome::Refused(JoinRefusal::QueueFull),
        },
    );
    feed.ok(
        4,
        DomainEvent::MlsJoinOutcome {
            attempt: 1,
            outcome: JoinOutcome::Pending {
                request_id: "r-1".into(),
                reason: PendingReason::AwaitingAdmin,
                expires_at: None,
                group_id: None,
            },
        },
    );
    let join = feed.store.mls_join();
    assert!(matches!(join.last, Some((2, JoinOutcome::Refused(_)))), "the newer attempt shows");
    assert!(join.records.contains_key("r-1"), "the pending request is never lost");
    assert_eq!(feed.store.mls_next_attempt(), 3);
}

#[test]
fn no_join_outcome_asks_for_another_join() {
    let outcomes = [
        JoinOutcome::Refused(JoinRefusal::RateLimited { retry_after_s: Some(10) }),
        JoinOutcome::Failed { code: "host_unreachable".into() },
        JoinOutcome::Unavailable,
        JoinOutcome::Joined { group_id: G.into() },
        JoinOutcome::Pending {
            request_id: "r".into(),
            reason: PendingReason::AwaitingMemberOnline,
            expires_at: None,
            group_id: None,
        },
    ];
    for outcome in outcomes {
        let mut feed = feed();
        feed.ok(2, DomainEvent::MlsJoinOutcome { attempt: 1, outcome });
        let asked = feed.store.mls_followups(Timestamp(100));
        assert!(
            !asked.iter().any(|k| matches!(k, IntentKind::MlsGroupJoin { .. })),
            "a join is never a follow-up"
        );
    }
}

#[test]
fn a_join_is_refused_while_another_is_in_flight_whatever_the_attempt() {
    let mut feed = feed();
    feed.ok(2, DomainEvent::MlsStarted(MlsStart::Join { attempt: 1 }));
    assert_eq!(feed.store.mls_join().in_flight, Some(1));
    let intent = Intent::human(IntentKind::MlsGroupJoin {
        link: InviteLink::new("rfinv-x"),
        attempt: feed.store.mls_next_attempt(),
    });
    assert!(matches!(
        handle_intent(&feed.store, intent, Timestamp(5)),
        Err(IntentError::MlsJoinBusy)
    ));
}

#[test]
fn an_event_post_and_a_history_row_with_one_log_seq_make_one_moment() {
    let mut feed = feed();
    history(&mut feed, HistoryDirection::Newest, vec![post(5, "p-5", "five")], 5, false);
    event(&mut feed, 100, Some(5), GroupEventKind::Post(Box::new(post(5, "p-5", "five"))));
    assert_eq!(moments(&feed).iter().filter(|m| m.ends_with(":5")).count(), 1);
    event(&mut feed, 101, Some(5), GroupEventKind::Post(Box::new(post(5, "p-5", "five"))));
    assert_eq!(moments(&feed).iter().filter(|m| m.ends_with(":5")).count(), 1);
}

#[test]
fn sent_after_an_event_moves_the_moment_without_a_duplicate() {
    let mut feed = feed();
    history(&mut feed, HistoryDirection::Newest, vec![post(5, "p-5", "five")], 5, false);
    started(&mut feed, "l-1");
    assert!(moments(&feed).iter().any(|m| m.ends_with(":l:l-1")));
    // The event arrives first, then the reply.
    event(&mut feed, 100, Some(6), GroupEventKind::Post(Box::new(mine(post(6, "p-6", "hi")))));
    outcome(&mut feed, "l-1", Some("p-6"), SendState::Sent { log_seq: 6 });
    let ids = moments(&feed);
    assert_eq!(ids.iter().filter(|m| m.contains(":6")).count(), 1, "{ids:?}");
    assert!(!ids.iter().any(|m| m.contains(":l:") || m.contains(":p:")), "{ids:?}");
}

#[test]
fn the_gap_fill_runs_one_round_at_a_time() {
    let mut feed = feed();
    history(&mut feed, HistoryDirection::Newest, vec![post(5, "p-5", "five")], 5, false);
    event(&mut feed, 100, Some(9), GroupEventKind::Post(Box::new(post(9, "p-9", "nine"))));
    let first = feed.store.mls_followups(Timestamp(1_000));
    assert!(first.iter().any(|k| matches!(
        k,
        IntentKind::MlsGroupHistory { direction: HistoryDirection::After(5), .. }
    )));
    // Once the source took the call, no second round starts, and a gap seen
    // meanwhile queues exactly one more.
    feed.ok(
        40,
        DomainEvent::MlsStarted(MlsStart::History {
            group_id: G.to_owned(),
            direction: HistoryDirection::After(5),
        }),
    );
    event(&mut feed, 101, Some(20), GroupEventKind::Post(Box::new(post(20, "p-20", "x"))));
    assert!(
        !feed
            .store
            .mls_followups(Timestamp(1_000))
            .iter()
            .any(|k| matches!(k, IntentKind::MlsGroupHistory { .. })),
        "one round at a time"
    );
    history(&mut feed, HistoryDirection::After(5), vec![post(6, "p-6", "six")], 20, false);
    assert!(
        feed.store
            .mls_followups(Timestamp(1_000))
            .iter()
            .any(|k| matches!(k, IntentKind::MlsGroupHistory { .. })),
        "the gap seen during the fill gets one more round"
    );
}

#[test]
fn the_ai_flag_stays_after_a_page_with_it_false() {
    let mut feed = feed();
    let ai = |flag: bool| {
        let mut p = post(5, "p-5", "five");
        p.author =
            Some(Author { anchor: "anc-s".into(), display: None, ai_flag: flag, is_me: false });
        p
    };
    history(&mut feed, HistoryDirection::Newest, vec![ai(true)], 5, false);
    assert!(feed.store.mls_is_ai(G, "anc-s"));
    history(&mut feed, HistoryDirection::Newest, vec![ai(false)], 5, false);
    assert!(feed.store.mls_is_ai(G, "anc-s"), "never dropped by a later row");
    let participant = feed.store.participants().find(|p| p.ai_flag);
    assert!(participant.is_some());
}

#[test]
fn a_not_sent_post_and_a_send_in_flight_survive_the_eviction_of_their_timeline() {
    let mut feed = feed();
    started(&mut feed, "l-1");
    outcome(
        &mut feed,
        "l-1",
        None,
        SendState::NotSent(NotSentReason::TooLarge { max_bytes: None }),
    );
    started(&mut feed, "l-2");
    // Three other groups are opened: the first timeline is evicted.
    for n in 2..=4 {
        let mut summary = mls_samples::summary();
        summary.group_id = format!("g-{n}");
        feed.ok(
            50 + n,
            DomainEvent::MlsGroupsListed { rows: vec![summary], cursor: None, first_page: false },
        );
        feed.ok(
            60 + n,
            DomainEvent::MlsHistoryPage {
                group_id: format!("g-{n}"),
                direction: HistoryDirection::Newest,
                posts: vec![],
                floor_log_seq: Some(1),
                last_log_seq: 0,
                more: false,
            },
        );
    }
    assert!(feed.store.mls_timeline(G).is_none(), "evicted");
    // The reply of the send still in flight lands all the same.
    outcome(&mut feed, "l-2", Some("p-2"), SendState::Queued(QueueReason::CatchingUp));
    started(&mut feed, "l-3");
    let own: Vec<_> = feed.store.mls_timeline(G).map(|t| t.own.clone()).unwrap_or_default();
    assert_eq!(own.len(), 3, "restored when the group opens again");
    assert!(own.iter().any(|i| i.local_id == "l-1" && matches!(i.send, SendState::NotSent(_))));
    assert!(own.iter().any(|i| i.local_id == "l-2" && i.post_id.as_deref() == Some("p-2")));
}

#[test]
fn an_own_row_that_arrives_while_the_send_runs_is_deduped_by_post_id() {
    let mut feed = feed();
    history(&mut feed, HistoryDirection::Newest, vec![], 0, false);
    started(&mut feed, "l-1");
    let mut pending = mine(post(0, "p-9", "hi"));
    pending.log_seq = None;
    pending.state = PostState::Pending;
    pending.send = Some(SendState::Queued(QueueReason::CatchingUp));
    history(&mut feed, HistoryDirection::After(0), vec![pending], 0, false);
    outcome(&mut feed, "l-1", Some("p-9"), SendState::Queued(QueueReason::CatchingUp));
    let own = feed.store.mls_timeline(G).map(|t| t.own.len()).unwrap_or_default();
    assert_eq!(own, 1, "one post, one bubble");
}

#[test]
fn a_used_up_daily_limit_locks_sends_until_it_resets_and_nothing_resends() {
    let mut feed = feed();
    started(&mut feed, "l-1");
    outcome(
        &mut feed,
        "l-1",
        None,
        SendState::NotSent(NotSentReason::RateLimited {
            resets_at: Some(Timestamp(5_000)),
            first_day: true,
            limit: 20,
            remaining: 0,
        }),
    );
    assert!(feed.store.mls_rate_locked_until(G, Timestamp(1_000)).is_some());
    assert!(feed.store.mls_rate_locked_until(G, Timestamp(6_000)).is_none(), "lifts at the reset");
    let send = |local: &str, retry: Option<&str>| {
        Intent::human(IntentKind::MlsGroupSend {
            group_id: G.to_owned(),
            text: "again".to_owned(),
            reply_to: None,
            mentions: Vec::new(),
            local_id: local.to_owned(),
            retry_of: retry.map(str::to_owned),
        })
    };
    assert!(matches!(
        handle_intent(&feed.store, send("l-2", Some("l-1")), Timestamp(1_000)),
        Err(IntentError::MlsRateLocked)
    ));
    // After the reset only a tap on Retry sends: the store asks for no send.
    assert!(handle_intent(&feed.store, send("l-2", Some("l-1")), Timestamp(6_000)).is_ok());
    assert!(
        !feed
            .store
            .mls_followups(Timestamp(6_000))
            .iter()
            .any(|k| matches!(k, IntentKind::MlsGroupSend { .. }))
    );
}

#[test]
fn a_lock_clears_every_record_and_refuses_later_writes() {
    let mut feed = feed();
    feed.ok(
        2,
        DomainEvent::AccountUpdated(
            serde_json::from_value(serde_json::json!({
                "id": "a-1", "label": "A", "lock_state": "Unlocked"
            }))
            .expect("an account"),
        ),
    );
    feed.store.bind_account(AccountId::new("a-1"));
    started(&mut feed, "l-1");
    feed.ok(
        3,
        DomainEvent::AccountLockChanged {
            account_id: AccountId::new("a-1"),
            lock_state: LockState::Locked,
        },
    );
    assert!(feed.store.mls_group(G).is_none() && feed.store.mls_timeline(G).is_none());
    // A write for the locked account changes nothing.
    feed.ok(4, DomainEvent::MlsJoinOutcome { attempt: 1, outcome: JoinOutcome::Unavailable });
    assert!(feed.store.mls_join().last.is_none());
}

#[test]
fn a_link_never_prints() {
    let link = InviteLink::new("rfinv-secret-link");
    let invite = mls_samples::invite();
    let effect = Effect::Call {
        method: "mls_group.join".into(),
        body: serde_json::json!({ "link": "rfinv-secret-link", "attempt": 1 }),
    };
    let intent = IntentKind::MlsGroupJoin { link: link.clone(), attempt: 1 };
    for text in
        [format!("{link:?}"), format!("{invite:?}"), format!("{effect:?}"), format!("{intent:?}")]
    {
        assert!(!text.contains("rfinv"), "{text}");
    }
}

#[test]
fn the_local_link_check_trims_and_refuses_what_is_not_printable_ascii() {
    assert_eq!(
        check_link("  rfinv-ok \n").map(|l| l.expose().to_owned()).as_deref(),
        Some("rfinv-ok")
    );
    assert!(check_link("has space").is_none());
    assert!(check_link(&"x".repeat(513)).is_none());
    assert!(check_link("").is_none());
    assert!(check_link("caf\u{e9}").is_none());
}

fn evict(feed: &mut Feed) {
    for n in 2..=4 {
        let mut summary = mls_samples::summary();
        summary.group_id = format!("g-{n}");
        feed.ok(
            50 + n,
            DomainEvent::MlsGroupsListed { rows: vec![summary], cursor: None, first_page: false },
        );
        feed.ok(
            60 + n,
            DomainEvent::MlsHistoryPage {
                group_id: format!("g-{n}"),
                direction: HistoryDirection::Newest,
                posts: vec![],
                floor_log_seq: Some(1),
                last_log_seq: 0,
                more: false,
            },
        );
    }
    assert!(feed.store.mls_timeline(G).is_none(), "evicted");
}

fn unlocked_account(feed: &mut Feed) {
    feed.ok(
        2,
        DomainEvent::AccountUpdated(
            serde_json::from_value(serde_json::json!({
                "id": "a-1", "label": "A", "lock_state": "Unlocked"
            }))
            .expect("an account"),
        ),
    );
    feed.store.bind_account(AccountId::new("a-1"));
}

#[test]
fn a_queued_post_that_fails_while_its_timeline_is_evicted_is_still_not_sent_on_reopen() {
    let mut feed = feed();
    started(&mut feed, "l-1");
    outcome(&mut feed, "l-1", Some("p-1"), SendState::Queued(QueueReason::CatchingUp));
    started(&mut feed, "l-2");
    outcome(&mut feed, "l-2", Some("p-2"), SendState::Delayed { send_after: Timestamp(9_000) });
    evict(&mut feed);
    event(
        &mut feed,
        100,
        None,
        GroupEventKind::PostState {
            post_id: "p-1".into(),
            state: SendState::NotSent(NotSentReason::Failed),
        },
    );
    started(&mut feed, "l-3");
    let own: Vec<_> = feed.store.mls_timeline(G).map(|t| t.own.clone()).unwrap_or_default();
    assert_eq!(own.len(), 3, "queued, delayed and new");
    let failed = own.iter().find(|i| i.local_id == "l-1").expect("the queued post is kept");
    assert!(matches!(failed.send, SendState::NotSent(NotSentReason::Failed)));
    assert_eq!(failed.text, "hi", "its text is kept");
    assert!(own.iter().any(|i| i.local_id == "l-2" && matches!(i.send, SendState::Delayed { .. })));
}

#[test]
fn locking_through_an_account_update_clears_the_mls_state() {
    let mut feed = feed();
    unlocked_account(&mut feed);
    started(&mut feed, "l-1");
    feed.store.mls_toggle_mention(G, "anc-1", "Ann");
    assert!(feed.store.mls_group(G).is_some() && feed.store.mls_timeline(G).is_some());
    feed.ok(
        3,
        DomainEvent::AccountUpdated(
            serde_json::from_value(serde_json::json!({
                "id": "a-1", "label": "A", "lock_state": "Locked"
            }))
            .expect("an account"),
        ),
    );
    assert!(feed.store.mls_group(G).is_none(), "the list is gone");
    assert!(feed.store.mls_timeline(G).is_none(), "the timeline is gone");
    assert!(feed.store.mls_mentions(G).is_empty(), "the picks are gone");
}

#[test]
fn mention_picks_belong_to_the_account_state_and_go_with_a_lock_or_a_removal() {
    let mut feed = feed();
    unlocked_account(&mut feed);
    feed.store.mls_toggle_mention(G, "anc-1", "Ann");
    feed.store.mls_toggle_mention(G, "anc-2", "Bob");
    assert_eq!(feed.store.mls_mentions(G).len(), 2);
    // Picking again unpicks.
    feed.store.mls_toggle_mention(G, "anc-1", "Ann");
    assert_eq!(feed.store.mls_mentions(G), vec![("anc-2".to_owned(), "Bob".to_owned())]);
    // A refused send keeps them; only an accepted one spends them.
    assert_eq!(feed.store.mls_mentions(G).len(), 1);
    feed.store.mls_mentions_spent(G);
    assert!(feed.store.mls_mentions(G).is_empty());
    // The limit holds.
    for n in 0..=MAX_MENTIONS {
        feed.store.mls_toggle_mention(G, &format!("a-{n}"), "x");
    }
    assert_eq!(feed.store.mls_mentions(G).len(), MAX_MENTIONS);
    // Another account's state starts with none, and a removal clears them.
    feed.ok(4, DomainEvent::AccountRemoved { account_id: AccountId::new("a-1") });
    assert!(feed.store.mls_mentions(G).is_empty());
}

#[test]
fn a_post_state_that_arrives_before_the_send_reply_is_applied_by_the_reply() {
    let mut feed = feed();
    started(&mut feed, "l-1");
    event(
        &mut feed,
        100,
        None,
        GroupEventKind::PostState {
            post_id: "p-1".into(),
            state: SendState::NotSent(NotSentReason::Failed),
        },
    );
    outcome(&mut feed, "l-1", Some("p-1"), SendState::Queued(QueueReason::CatchingUp));
    let own = feed.store.mls_timeline(G).map(|t| t.own.clone()).unwrap_or_default();
    assert!(matches!(own[0].send, SendState::NotSent(NotSentReason::Failed)));
}
