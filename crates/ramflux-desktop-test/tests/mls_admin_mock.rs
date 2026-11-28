// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The M3 answers of the demo MLS groups, run through the policy, the mock
//! and the store the way the app does: a call is decided, the store is told
//! the source took it, and the answers are applied in delay order.

use ramflux_desktop_core::mls_group::*;
use ramflux_desktop_core::*;
use ramflux_desktop_test::mock_mls::MockMls;

struct Rig {
    store: Store,
    mock: MockMls,
    seq: u64,
    now: i64,
}

impl Rig {
    fn new() -> Self {
        let mut rig = Self { store: Store::new(), mock: MockMls::new(), seq: 0, now: 1_000_000 };
        rig.apply(DomainEvent::AccountUpdated(
            serde_json::from_value(serde_json::json!({
                "id": "a-1", "label": "A", "lock_state": "Unlocked"
            }))
            .expect("an account"),
        ));
        rig.store.bind_account(AccountId::new("a-1"));
        // The list, as the app asks for it first.
        rig.call(IntentKind::MlsGroupList { cursor: None }).expect("the list passes");
        rig
    }

    fn apply(&mut self, event: DomainEvent) {
        self.seq += 1;
        self.store
            .apply(&EventEnvelope::new(self.seq, Timestamp(self.now), event))
            .expect("the event applies");
    }

    /// Decides the intent, tells the store the call started, and applies the
    /// answers in delay order (as the clock moves).
    fn call(&mut self, kind: IntentKind) -> Result<(), IntentError> {
        let effects = handle_intent(&self.store, Intent::human(kind), Timestamp(self.now))?;
        let effect = effects.first().expect("one call");
        let method = effect.method().expect("method").to_owned();
        let body = effect.body().cloned().expect("body");
        if let Some(started) = DomainEvent::mls_started(&method, &body) {
            self.apply(started);
        }
        let mut answers = self.mock.respond(&method, &body, Timestamp(self.now));
        answers.sort_by_key(|(delay, _)| *delay);
        for (delay, event) in answers {
            self.now += i64::try_from(delay).unwrap_or(0);
            self.apply(event);
        }
        Ok(())
    }

    fn info(&mut self, group: &str) {
        self.call(IntentKind::MlsGroupInfo { group_id: group.to_owned() }).expect("info");
    }

    fn members(&mut self, group: &str) {
        self.call(IntentKind::MlsGroupMembers { group_id: group.to_owned(), cursor: None })
            .expect("members");
    }

    fn history(&mut self, group: &str) {
        self.call(IntentKind::MlsGroupHistory {
            group_id: group.to_owned(),
            direction: HistoryDirection::Newest,
        })
        .expect("history");
    }
}

const G: &str = "g-design";

fn anchor_of(rig: &Rig, group: &str, name: &str) -> OpaqueId {
    let row = rig
        .store
        .mls_members(group)
        .and_then(|p| p.rows.values().find(|r| r.display.as_deref() == Some(name)))
        .unwrap_or_else(|| panic!("a member named {name}"));
    OpaqueId::new(row.anchor.clone())
}

#[test]
fn approving_ana_removes_her_row_only_after_the_reply_and_the_joined_row() {
    let mut rig = Rig::new();
    rig.info(G);
    rig.call(IntentKind::MlsRequestsList { group_id: G.to_owned(), cursor: None }).expect("list");
    let pager = rig.store.mls_requests(G).expect("loaded");
    assert_eq!(pager.rows.len(), 8, "seven rows and Fay, who arrived by event");
    rig.call(IntentKind::MlsRequestApprove {
        group_id: G.to_owned(),
        request_id: OpaqueId::new("rq-ana"),
    })
    .expect("approve passes");
    let pager = rig.store.mls_requests(G).expect("loaded");
    assert!(pager.row(&OpaqueId::new("rq-ana")).is_none(), "gone after the event");
    assert!(matches!(rig.store.mls_admin_state(G), AdminState::Idle));
    // Ben is not found, Dana is paced.
    rig.call(IntentKind::MlsRequestApprove {
        group_id: G.to_owned(),
        request_id: OpaqueId::new("rq-ben"),
    })
    .expect("approve passes");
    assert!(rig.store.mls_requests(G).expect("loaded").row(&OpaqueId::new("rq-ben")).is_none());
    rig.call(IntentKind::MlsRequestApprove {
        group_id: G.to_owned(),
        request_id: OpaqueId::new("rq-dana"),
    })
    .expect("approve passes");
    assert!(rig.store.mls_paced_until(G, Timestamp(rig.now)).is_some());
    assert!(matches!(
        handle_intent(
            &rig.store,
            Intent::human(IntentKind::MlsRequestApprove {
                group_id: G.to_owned(),
                request_id: OpaqueId::new("rq-eve"),
            }),
            Timestamp(rig.now),
        ),
        Err(IntentError::MlsPaced)
    ));
}

#[test]
fn the_role_change_of_ben_has_its_event_before_the_reply_and_leaves_no_stuck_chip() {
    let mut rig = Rig::new();
    rig.info(G);
    rig.members(G);
    let ben = anchor_of(&rig, G, "Ben");
    rig.call(IntentKind::MlsRoleSet { group_id: G.to_owned(), anchor: ben.clone(), admin: true })
        .expect("role passes");
    assert!(matches!(rig.store.mls_admin_state(G), AdminState::Idle));
    let role =
        rig.store.mls_members(G).and_then(|p| p.rows.get(ben.expose())).map(|r| r.role.clone());
    assert_eq!(role, Some(MlsRole::Admin));
}

#[test]
fn removing_dana_and_banning_eve_leave_the_roster_and_free_the_slot() {
    let mut rig = Rig::new();
    rig.info(G);
    rig.members(G);
    for (name, ban) in [("Dana", false), ("Eve", true)] {
        let anchor = anchor_of(&rig, G, name);
        let kind = if ban {
            IntentKind::MlsMemberBan { group_id: G.to_owned(), anchor: anchor.clone() }
        } else {
            IntentKind::MlsMemberRemove { group_id: G.to_owned(), anchor: anchor.clone() }
        };
        rig.call(kind).expect("passes");
        assert!(matches!(rig.store.mls_admin_state(G), AdminState::Idle), "{name}");
        assert!(rig.store.mls_members(G).is_some_and(|p| !p.rows.contains_key(anchor.expose())));
    }
}

#[test]
fn the_head_commit_of_design_is_250_and_an_undo_marks_it_undone() {
    let mut rig = Rig::new();
    rig.info(G);
    rig.history(G);
    assert_eq!(rig.store.mls_head_commit(G), Some(250));
    rig.call(IntentKind::MlsVoid { group_id: G.to_owned(), log_seq: 250 }).expect("void passes");
    assert!(matches!(rig.store.mls_admin_state(G), AdminState::Idle));
    assert!(rig.store.mls_voided(G).contains(&250));
    assert_eq!(rig.store.mls_head_commit(G), None, "the void row is the newest commit");
    // An older commit row is never offered.
    assert!(matches!(
        handle_intent(
            &rig.store,
            Intent::human(IntentKind::MlsVoid { group_id: G.to_owned(), log_seq: 240 }),
            Timestamp(rig.now),
        ),
        Err(IntentError::MlsVoidTarget)
    ));
}

#[test]
fn a_void_on_the_stale_head_group_is_refused_with_a_gap_fill() {
    let mut rig = Rig::new();
    let group = "g-stale";
    rig.info(group);
    rig.history(group);
    assert_eq!(rig.store.mls_head_commit(group), Some(250));
    rig.call(IntentKind::MlsVoid { group_id: group.to_owned(), log_seq: 250 }).expect("passes");
    assert!(matches!(rig.store.mls_admin_state(group), AdminState::Idle));
    assert_eq!(rig.store.mls_admin_failure(group).map(|f| f.code.as_str()), Some("void_not_head"));
    assert_eq!(rig.store.mls_head_commit(group), Some(252), "the newer commit moved the head");
}

#[test]
fn the_first_refresh_of_design_brings_the_warnings_in_turn() {
    let mut rig = Rig::new();
    rig.info(G);
    let answers = {
        let now = Timestamp(rig.now);
        rig.mock.respond(methods::MLS_GROUP_REFRESH, &serde_json::json!({ "group_id": G }), now)
    };
    let warnings = answers
        .iter()
        .filter(|(_, e)| {
            matches!(e, DomainEvent::MlsGroupEvent { kind: GroupEventKind::Warning { .. }, .. })
        })
        .count();
    assert_eq!(warnings, 12 * 2 + 3, "each set and cleared, then three together");
    // The F2-7 news wait for the second refresh.
    assert!(!answers.iter().any(|(_, e)| matches!(
        e,
        DomainEvent::MlsGroupEvent { kind: GroupEventKind::Membership(_), .. }
    )));
}

#[test]
fn quotas_change_only_after_the_next_group_read() {
    let mut rig = Rig::new();
    rig.info(G);
    let before = rig.store.mls_group(G).and_then(|g| g.quota_post.clone()).expect("a quota");
    assert_eq!((before.limit, before.remaining), (500, 60));
    rig.call(IntentKind::MlsGroupSend {
        group_id: G.to_owned(),
        text: "hi".to_owned(),
        reply_to: None,
        mentions: Vec::new(),
        local_id: "l-1".to_owned(),
        retry_of: None,
    })
    .expect("send passes");
    // The send asks for a group read; it has not happened yet.
    assert_eq!(
        rig.store.mls_group(G).and_then(|g| g.quota_post.clone()).map(|q| q.remaining),
        Some(60)
    );
    assert!(
        rig.store
            .mls_followups(Timestamp(rig.now + 5_000))
            .iter()
            .any(|k| matches!(k, IntentKind::MlsGroupInfo { .. }))
    );
    rig.info(G);
    assert_eq!(
        rig.store.mls_group(G).and_then(|g| g.quota_post.clone()).map(|q| q.remaining),
        Some(59)
    );
}

#[test]
fn the_queue_has_two_pages_and_the_second_fails_once() {
    let mut rig = Rig::new();
    let group = "g-queue";
    rig.info(group);
    rig.call(IntentKind::MlsRequestsList { group_id: group.to_owned(), cursor: None })
        .expect("page 1");
    let pager = rig.store.mls_requests(group).expect("loaded");
    assert_eq!((pager.rows.len(), pager.cursor.as_deref()), (200, Some("r2")));
    rig.call(IntentKind::MlsRequestsList {
        group_id: group.to_owned(),
        cursor: Some("r2".to_owned()),
    })
    .expect("page 2");
    let pager = rig.store.mls_requests(group).expect("loaded");
    assert_eq!(pager.page_error.as_deref(), Some("host_unreachable"));
    rig.call(IntentKind::MlsRequestsList {
        group_id: group.to_owned(),
        cursor: Some("r2".to_owned()),
    })
    .expect("retry");
    let pager = rig.store.mls_requests(group).expect("loaded");
    assert_eq!((pager.rows.len(), pager.cursor.clone()), (230, None));
}

#[test]
fn the_late_skip_of_the_late_catch_up_group_changes_nothing() {
    let mut rig = Rig::new();
    let group = "g-latecatch";
    rig.info(group);
    assert_eq!(rig.store.mls_catch_up(group).map(|r| r.state), Some(CatchUpState::Running));
    rig.call(IntentKind::MlsCatchUpSkip { group_id: group.to_owned() }).expect("skip passes");
    // The idle event came first; the reply, 3 s after the call, changed nothing.
    assert_eq!(rig.store.mls_catch_up(group).map(|r| r.state), Some(CatchUpState::Idle));
}
