// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The MLS admin owner (MLS GUI batch M3): the admin slot, pending commits
//! and the event that came before its reply, the epoch, the warning merge
//! rule, the late skip, the requests page generation, the head-commit rule
//! and the redaction of ids.

mod common;
mod mls_samples;

use common::*;
use ramflux_desktop_core::mls_group::*;
use ramflux_desktop_core::*;
use serde_json::Value;

const G: &str = "g-1";

fn member(anchor: &str, name: &str, role: MlsRole, me: bool) -> MemberRow {
    MemberRow {
        anchor: anchor.to_owned(),
        display: Some(name.to_owned()),
        role,
        ai_flag: false,
        devices: 1,
        is_me: me,
    }
}

/// A feed whose account is unlocked and owns `G` (approval on) with three
/// members loaded.
fn feed() -> Feed {
    let mut feed = Feed::new();
    feed.ok(
        1,
        DomainEvent::AccountUpdated(
            serde_json::from_value(serde_json::json!({
                "id": "a-1", "label": "A", "lock_state": "Unlocked"
            }))
            .expect("an account"),
        ),
    );
    feed.store.bind_account(AccountId::new("a-1"));
    let mut summary = mls_samples::summary();
    summary.group_id = G.to_owned();
    summary.catch_up = None;
    summary.warnings.clear();
    feed.ok(
        2,
        DomainEvent::MlsGroupsListed { rows: vec![summary], cursor: None, first_page: true },
    );
    feed.ok(
        3,
        DomainEvent::MlsMembersPage {
            group_id: G.to_owned(),
            rows: vec![
                member("anc-me", "Me", MlsRole::Owner, true),
                member("anc-ben", "Ben", MlsRole::Member, false),
                member("anc-chen", "Chen", MlsRole::Admin, false),
                member("anc-eve", "Eve", MlsRole::Member, false),
            ],
            cursor: None,
            failed: None,
        },
    );
    feed
}

/// Asks the policy for an intent and, when it passes, tells the store the
/// source took the call.
fn ask(feed: &mut Feed, kind: IntentKind) -> Result<(String, Value), IntentError> {
    let effects = handle_intent(&feed.store, Intent::human(kind), Timestamp(1_000))?;
    let effect = effects.first().expect("one call");
    let method = effect.method().expect("a method").to_owned();
    let body = effect.body().cloned().expect("a body");
    if let Some(started) = DomainEvent::mls_started(&method, &body) {
        feed.ok(40, started);
    }
    Ok((method, body))
}

fn anchor(id: &str) -> OpaqueId {
    OpaqueId::new(id)
}

fn remove(feed: &mut Feed, who: &str) -> Result<(String, Value), IntentError> {
    ask(feed, IntentKind::MlsMemberRemove { group_id: G.to_owned(), anchor: anchor(who) })
}

fn ids(body: &Value) -> (u64, u64) {
    (body["epoch"].as_u64().expect("epoch"), body["action_id"].as_u64().expect("action id"))
}

fn outcome(
    feed: &mut Feed,
    body: &Value,
    kind: AdminKind,
    target: AdminTarget,
    result: AdminResult,
) {
    let (epoch, action_id) = ids(body);
    feed.ok(
        50,
        DomainEvent::MlsAdminOutcome {
            epoch,
            action_id,
            group_id: G.to_owned(),
            kind,
            target,
            result,
        },
    );
}

fn commit_post(log_seq: u64, kind: PostKind, body: PostBody) -> Post {
    Post {
        log_seq: Some(log_seq),
        post_id: format!("c-{log_seq}"),
        author: None,
        appended_at: Some(Timestamp(log_seq as i64)),
        kind,
        body,
        reply_to: None,
        mentions: Vec::new(),
        state: PostState::Sent,
        send: None,
    }
}

fn change(kind: MembershipKind, who: &str, role: Option<MlsRole>) -> MembershipChange {
    MembershipChange {
        kind,
        anchor: who.to_owned(),
        display: None,
        ai_flag: false,
        role,
        is_me: false,
    }
}

fn membership_event(feed: &mut Feed, seq: u64, log_seq: u64, changes: Vec<MembershipChange>) {
    let post =
        commit_post(log_seq, PostKind::Membership, PostBody::Membership { changes, by: None });
    feed.ok(
        60,
        DomainEvent::MlsGroupEvent {
            seq,
            group_id: G.to_owned(),
            log_seq: Some(log_seq),
            kind: GroupEventKind::Membership(Box::new(post)),
        },
    );
}

fn warning(feed: &mut Feed, seq: u64, code: WarningCode, cleared: bool) {
    feed.ok(
        61,
        DomainEvent::MlsGroupEvent {
            seq,
            group_id: G.to_owned(),
            log_seq: None,
            kind: GroupEventKind::Warning { code, cleared },
        },
    );
}

#[test]
fn a_second_admin_action_on_a_busy_group_is_refused_and_a_typed_refusal_frees_the_slot() {
    let mut feed = feed();
    let (_, body) = remove(&mut feed, "anc-ben").expect("the first action passes");
    assert!(feed.store.mls_admin_busy(G));
    let second = ask(
        &mut feed,
        IntentKind::MlsMemberBan { group_id: G.to_owned(), anchor: anchor("anc-eve") },
    );
    assert!(matches!(second, Err(IntentError::MlsAdminBusy)));
    outcome(
        &mut feed,
        &body,
        AdminKind::Remove,
        AdminTarget::Member { anchor: anchor("anc-ben"), admin: None },
        AdminResult::Refused {
            code: "host_unreachable".into(),
            retry_at: None,
            head_log_seq: None,
        },
    );
    assert!(!feed.store.mls_admin_busy(G), "the slot is free");
    assert_eq!(feed.store.mls_admin_failure(G).map(|f| f.code.as_str()), Some("host_unreachable"));
    assert!(
        ask(
            &mut feed,
            IntentKind::MlsMemberBan { group_id: G.to_owned(), anchor: anchor("anc-eve") }
        )
        .is_ok()
    );
}

#[test]
fn the_reply_then_the_event_resolves_a_commit() {
    let mut feed = feed();
    let (_, body) = remove(&mut feed, "anc-ben").expect("passes");
    let target = AdminTarget::Member { anchor: anchor("anc-ben"), admin: None };
    outcome(&mut feed, &body, AdminKind::Remove, target, AdminResult::Committed { log_seq: 311 });
    assert!(matches!(feed.store.mls_admin_state(G), AdminState::Updating(_)));
    assert!(
        feed.store.mls_members(G).is_some_and(|p| p.rows.contains_key("anc-ben")),
        "the row stays until the event"
    );
    membership_event(&mut feed, 100, 311, vec![change(MembershipKind::Removed, "anc-ben", None)]);
    assert!(matches!(feed.store.mls_admin_state(G), AdminState::Idle));
    assert!(feed.store.mls_members(G).is_some_and(|p| !p.rows.contains_key("anc-ben")));
}

#[test]
fn an_event_that_came_before_its_reply_resolves_the_action_at_once() {
    let mut feed = feed();
    let (_, body) = ask(
        &mut feed,
        IntentKind::MlsRoleSet { group_id: G.to_owned(), anchor: anchor("anc-ben"), admin: true },
    )
    .expect("passes");
    membership_event(
        &mut feed,
        100,
        310,
        vec![change(MembershipKind::RoleChanged, "anc-ben", Some(MlsRole::Admin))],
    );
    assert!(matches!(feed.store.mls_admin_state(G), AdminState::Running(_)), "still waiting");
    let target = AdminTarget::Member { anchor: anchor("anc-ben"), admin: Some(true) };
    outcome(&mut feed, &body, AdminKind::Role, target, AdminResult::Committed { log_seq: 310 });
    assert!(matches!(feed.store.mls_admin_state(G), AdminState::Idle), "no stuck Updating");
    let role =
        feed.store.mls_members(G).and_then(|p| p.rows.get("anc-ben")).map(|r| r.role.clone());
    assert_eq!(role, Some(MlsRole::Admin));
}

#[test]
fn a_result_of_an_old_epoch_after_a_lock_and_an_unlock_is_dropped() {
    let mut feed = feed();
    let before = feed.store.mls_epoch();
    let lock = |state: &str| {
        DomainEvent::AccountUpdated(
            serde_json::from_value(serde_json::json!({
                "id": "a-1", "label": "A", "lock_state": state
            }))
            .expect("an account"),
        )
    };
    feed.ok(5, lock("Locked"));
    feed.ok(6, lock("Unlocked"));
    assert!(feed.store.mls_epoch() > before, "the epoch moved");
    assert!(feed.store.mls_group(G).is_none(), "nothing survived the lock");
    // The account is back: the list and the members load again.
    let mut summary = mls_samples::summary();
    summary.group_id = G.to_owned();
    feed.ok(
        7,
        DomainEvent::MlsGroupsListed { rows: vec![summary], cursor: None, first_page: true },
    );
    feed.ok(
        8,
        DomainEvent::MlsMembersPage {
            group_id: G.to_owned(),
            rows: vec![member("anc-ben", "Ben", MlsRole::Member, false)],
            cursor: None,
            failed: None,
        },
    );
    let (_, body) = remove(&mut feed, "anc-ben").expect("passes");
    let (epoch, action_id) = ids(&body);
    assert_eq!(epoch, feed.store.mls_epoch());
    // A late result of the old epoch with the same action id changes nothing.
    feed.ok(
        9,
        DomainEvent::MlsAdminOutcome {
            epoch: before,
            action_id,
            group_id: G.to_owned(),
            kind: AdminKind::Remove,
            target: AdminTarget::Member { anchor: anchor("anc-ben"), admin: None },
            result: AdminResult::Refused {
                code: "generic".into(),
                retry_at: None,
                head_log_seq: None,
            },
        },
    );
    assert!(feed.store.mls_admin_busy(G), "the new action still holds the slot");
    assert!(feed.store.mls_admin_failure(G).is_none());
}

#[test]
fn a_stale_info_read_does_not_bring_back_a_cleared_warning() {
    let mut feed = feed();
    let issued = feed.store.mls_event_gen(G);
    warning(&mut feed, 100, WarningCode::ForkAlarm { head_n: 2 }, false);
    assert_eq!(feed.store.mls_warnings(G).len(), 1);
    warning(&mut feed, 101, WarningCode::ForkAlarm { head_n: 2 }, true);
    assert!(feed.store.mls_warnings(G).is_empty());
    // The read was issued before the clear and still lists the warning.
    let mut stale = mls_samples::summary();
    stale.group_id = G.to_owned();
    stale.catch_up = None;
    stale.warnings = vec![Warning {
        code: "fork_alarm".to_owned(),
        params: serde_json::json!({ "head_n": 2 }),
        cleared: false,
    }];
    feed.ok(70, DomainEvent::MlsGroupInfoRead { summary: Box::new(stale.clone()), issued });
    assert!(feed.store.mls_warnings(G).is_empty(), "the clear stays");
    // A read issued after the clear is the truth.
    let now = feed.store.mls_event_gen(G);
    feed.ok(71, DomainEvent::MlsGroupInfoRead { summary: Box::new(stale), issued: now });
    assert_eq!(feed.store.mls_warnings(G).len(), 1);
}

#[test]
fn a_late_skip_result_after_an_idle_event_changes_nothing() {
    let mut feed = feed();
    let running = |feed: &mut Feed, seq: u64, state: CatchUpState| {
        feed.ok(
            80,
            DomainEvent::MlsGroupEvent {
                seq,
                group_id: G.to_owned(),
                log_seq: None,
                kind: GroupEventKind::CatchUp { state, done: 5, total: 10 },
            },
        );
    };
    running(&mut feed, 100, CatchUpState::Running);
    let (_, body) =
        ask(&mut feed, IntentKind::MlsCatchUpSkip { group_id: G.to_owned() }).expect("passes");
    assert_eq!(body["skip"], Value::Bool(true));
    running(&mut feed, 101, CatchUpState::Idle);
    feed.ok(
        81,
        DomainEvent::MlsSkipOutcome {
            epoch: feed.store.mls_epoch(),
            group_id: G.to_owned(),
            from_seq: body["from_seq"].as_u64().expect("seq"),
            result: SkipResult::Rejoining,
        },
    );
    assert_eq!(feed.store.mls_catch_up(G).map(|r| r.state), Some(CatchUpState::Idle));
    // Skip is only for a running catch-up.
    assert!(matches!(
        ask(&mut feed, IntentKind::MlsCatchUpSkip { group_id: G.to_owned() }),
        Err(IntentError::MlsNotCatchingUp)
    ));
}

#[test]
fn rejoining_to_idle_asks_for_the_newest_page_and_a_group_read() {
    let mut feed = feed();
    for (seq, state) in [(100, CatchUpState::Rejoining), (101, CatchUpState::Idle)] {
        feed.ok(
            82,
            DomainEvent::MlsGroupEvent {
                seq,
                group_id: G.to_owned(),
                log_seq: None,
                kind: GroupEventKind::CatchUp { state, done: 0, total: 0 },
            },
        );
    }
    let asked = feed.store.mls_followups(Timestamp(100_000));
    assert!(asked.iter().any(|k| matches!(
        k,
        IntentKind::MlsGroupHistory { direction: HistoryDirection::Newest, .. }
    )));
    assert!(asked.iter().any(|k| matches!(k, IntentKind::MlsGroupInfo { .. })));
}

#[test]
fn a_requests_page_of_an_older_generation_is_dropped() {
    let mut feed = feed();
    let (_, body) =
        ask(&mut feed, IntentKind::MlsRequestsList { group_id: G.to_owned(), cursor: None })
            .expect("passes");
    let gen_asked = body["page_gen"].as_u64().expect("gen");
    // Approval goes off: the list resets.
    let settings = commit_post(
        5,
        PostKind::Settings,
        PostBody::Settings {
            change: SettingsChange { name: None, approval: Some(false) },
            by: None,
        },
    );
    feed.ok(
        83,
        DomainEvent::MlsGroupEvent {
            seq: 100,
            group_id: G.to_owned(),
            log_seq: Some(5),
            kind: GroupEventKind::Settings(Box::new(settings)),
        },
    );
    feed.ok(
        84,
        DomainEvent::MlsRequestsPage {
            group_id: G.to_owned(),
            page_gen: gen_asked,
            first_page: true,
            rows: vec![mls_samples::join_request("r-1", "anc-r1")],
            cursor: None,
            failed: None,
        },
    );
    assert!(feed.store.mls_requests(G).is_none_or(|p| p.rows.is_empty() && !p.loaded));
    assert!(matches!(
        ask(&mut feed, IntentKind::MlsRequestsList { group_id: G.to_owned(), cursor: None }),
        Err(IntentError::MlsApprovalOff)
    ));
}

#[test]
fn the_head_commit_is_the_newest_commit_row_and_never_a_void_row() {
    let mut feed = feed();
    let rows = vec![
        commit_post(
            240,
            PostKind::Membership,
            PostBody::Membership {
                changes: vec![change(MembershipKind::Joined, "anc-ben", None)],
                by: None,
            },
        ),
        commit_post(
            250,
            PostKind::Settings,
            PostBody::Settings {
                change: SettingsChange { name: Some("Team".into()), approval: None },
                by: None,
            },
        ),
    ];
    assert_eq!(feed.store.mls_head_commit(G), None, "no page is loaded");
    feed.ok(
        90,
        DomainEvent::MlsHistoryPage {
            group_id: G.to_owned(),
            direction: HistoryDirection::Newest,
            posts: rows,
            floor_log_seq: Some(1),
            last_log_seq: 250,
            more: false,
        },
    );
    assert_eq!(feed.store.mls_head_commit(G), Some(250));
    // A void row of the head: no head any more.
    let void =
        commit_post(251, PostKind::Void, PostBody::Void { voided_log_seq: Some(250), by: None });
    feed.ok(
        91,
        DomainEvent::MlsGroupEvent {
            seq: 100,
            group_id: G.to_owned(),
            log_seq: Some(251),
            kind: GroupEventKind::Void(Box::new(void)),
        },
    );
    assert_eq!(feed.store.mls_head_commit(G), None);
    assert!(feed.store.mls_voided(G).contains(&250));
}

#[test]
fn no_head_commit_while_the_window_is_cut_at_the_newest_end() {
    let mut feed = feed();
    let mut rows: Vec<Post> = (1..=600)
        .map(|n| Post {
            log_seq: Some(n),
            post_id: format!("t-{n}"),
            body: PostBody::Text("x".into()),
            ..mls_samples::post()
        })
        .collect();
    rows.push(commit_post(
        700,
        PostKind::Settings,
        PostBody::Settings {
            change: SettingsChange { name: Some("Team".into()), approval: None },
            by: None,
        },
    ));
    feed.ok(
        92,
        DomainEvent::MlsHistoryPage {
            group_id: G.to_owned(),
            direction: HistoryDirection::Newest,
            posts: vec![rows[600].clone()],
            floor_log_seq: Some(1),
            last_log_seq: 700,
            more: true,
        },
    );
    assert_eq!(feed.store.mls_head_commit(G), Some(700));
    feed.ok(
        93,
        DomainEvent::MlsHistoryPage {
            group_id: G.to_owned(),
            direction: HistoryDirection::Before(700),
            posts: rows[..600].to_vec(),
            floor_log_seq: Some(1),
            last_log_seq: 700,
            more: false,
        },
    );
    assert_eq!(feed.store.mls_head_commit(G), None, "the newest rows were cut");
}

#[test]
fn a_pending_join_is_listed_once_and_only_for_an_admin_of_an_approval_group() {
    let mut feed = feed();
    ask(&mut feed, IntentKind::MlsRequestsList { group_id: G.to_owned(), cursor: None })
        .expect("passes");
    let page_gen = feed.store.mls_requests(G).map_or(0, |p| p.page_gen);
    feed.ok(
        94,
        DomainEvent::MlsRequestsPage {
            group_id: G.to_owned(),
            page_gen,
            first_page: true,
            rows: vec![mls_samples::join_request("r-1", "anc-r1")],
            cursor: None,
            failed: None,
        },
    );
    let fay = mls_samples::join_request("r-fay", "anc-fay");
    for seq in [100, 101] {
        feed.ok(
            95,
            DomainEvent::MlsGroupEvent {
                seq,
                group_id: G.to_owned(),
                log_seq: None,
                kind: GroupEventKind::PendingJoin(Box::new(fay.clone())),
            },
        );
    }
    assert_eq!(feed.store.mls_requests(G).map_or(0, |p| p.rows.len()), 2);
}

#[test]
fn only_the_three_codes_can_be_hidden_and_a_new_warning_shows_again() {
    let mut feed = feed();
    warning(&mut feed, 100, WarningCode::ForkAlarm { head_n: 1 }, false);
    warning(&mut feed, 101, WarningCode::HostUnusual { kind: "late_roll".into() }, false);
    feed.store.mls_hide_banner(G, "fork_alarm");
    feed.store.mls_hide_banner(G, "host_unusual");
    assert!(!feed.store.mls_banner_hidden(G, "fork_alarm"));
    assert!(feed.store.mls_banner_hidden(G, "host_unusual"));
    warning(&mut feed, 102, WarningCode::HostUnusual { kind: "late_roll".into() }, true);
    warning(&mut feed, 103, WarningCode::HostUnusual { kind: "late_roll".into() }, false);
    assert!(!feed.store.mls_banner_hidden(G, "host_unusual"));
}

#[test]
fn a_member_never_sees_head_commit_rejected_and_the_menu_follows_the_role() {
    let mut feed = feed();
    warning(&mut feed, 100, WarningCode::HeadCommitRejected { log_seq: 250 }, false);
    assert_eq!(feed.store.mls_warnings(G).len(), 1, "the owner sees it");
    let mut member_view = mls_samples::summary();
    member_view.group_id = G.to_owned();
    member_view.my_role = MlsRole::Member;
    member_view.catch_up = None;
    feed.ok(
        96,
        DomainEvent::MlsGroupsListed { rows: vec![member_view], cursor: None, first_page: true },
    );
    assert!(feed.store.mls_warnings(G).is_empty(), "a member does not");
    assert!(matches!(remove(&mut feed, "anc-ben"), Err(IntentError::MlsNotAllowed)));
}

#[test]
fn ids_never_print() {
    let request = mls_samples::join_request("r-secret", "anc-secret");
    let text = format!(
        "{request:?} {:?} {:?}",
        anchor("anc-secret"),
        AdminTarget::Request(anchor("r-secret"))
    );
    assert!(!text.contains("secret") && !text.contains("Ana"), "{text}");
    let intent = IntentKind::MlsMemberBan { group_id: G.to_owned(), anchor: anchor("anc-secret") };
    assert!(!format!("{intent:?}").contains("anc-secret"));
}
