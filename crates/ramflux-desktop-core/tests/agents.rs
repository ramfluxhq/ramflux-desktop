// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Named agents (AGENT-IDENTITY-01) in policy and in the store: every step
//! is the owner's, on an unlocked, connected person's account; creation
//! carries both limits and never a host; changes wait for the owner's
//! confirmation; retired agents are read only; the runtime key never
//! reaches a body or a serialized intent.

mod common;

use common::*;
use ramflux_desktop_core::*;
use serde_json::json;

fn account(id: &str, kind: AccountKind, lock: LockState, networking: Networking) -> Account {
    Account {
        id: AccountId::new(id),
        label: id.to_owned(),
        kind,
        node_id: None,
        device_id: None,
        lock_state: lock,
        self_participant_id: None,
        secure_sessions: SecureSessions::Ready,
        networking_session: None,
        networking_custody: None,
        authority_blocked: false,
        networking_failure: None,
        networking_reason: None,
        networking,
    }
}

fn world() -> Feed {
    let mut feed = Feed::new();
    feed.ok(
        1,
        DomainEvent::CapabilitiesUpdated(Capabilities {
            born_v3_creation: true,
            ..Capabilities::default()
        }),
    );
    feed.ok(
        2,
        DomainEvent::AccountUpdated(account(
            "owner",
            AccountKind::HumanInteractive,
            LockState::Unlocked,
            Networking::Live,
        )),
    );
    feed.ok(
        3,
        DomainEvent::AccountUpdated(account(
            "locked",
            AccountKind::HumanInteractive,
            LockState::Locked,
            Networking::Live,
        )),
    );
    feed
}

fn record(anchor: &str, lifecycle: AgentLifecycle, owner: &str) -> AgentRecord {
    AgentRecord {
        agent_anchor: anchor.into(),
        custodian_anchor: owner.into(),
        persona: AgentPersona {
            name: "Research helper".into(),
            avatar: "preset:spark:teal".into(),
            responsibility: "Reads papers".into(),
        },
        lifecycle,
        access: AgentAccess::owner_only(),
        role: "named".into(),
        base: json!({"id": "base-1"}),
        runtime_name: format!("a_{anchor}"),
    }
}

fn with_agent(mut feed: Feed, agent: AgentRecord) -> Feed {
    feed.ok(4, DomainEvent::AgentRecordUpdated(Box::new(agent)));
    feed
}

fn decide(store: &Store, kind: IntentKind) -> Result<Vec<Effect>, IntentError> {
    handle_intent(store, Intent::human(kind), Timestamp(10))
}

fn confirmed(kind: IntentKind) -> Intent {
    Intent::human(kind).confirmed_by_human(ConfirmationIssuer::new().issue_tap())
}

fn request() -> AgentCreateRequest {
    AgentCreateRequest {
        persona: AgentPersona {
            name: "Research helper".into(),
            avatar: "preset:spark:teal".into(),
            responsibility: "Reads papers".into(),
        },
        config: AgentConfig {
            provider: "anthropic".into(),
            model: "model-x".into(),
            autonomy: "full".into(),
            tools: AgentTools::None,
            prompt: None,
            max_concurrent_turns: 2,
            max_cost_per_day_cents: 500,
        },
        access: AgentAccess::owner_only(),
    }
}

#[test]
fn an_agent_never_changes_an_agent() {
    let store = world().store;
    let attempt = handle_intent(
        &store,
        Intent::agent(
            ParticipantId::new("p-bot"),
            IntentKind::AgentDelete { agent: "agent-1".into() },
        ),
        Timestamp(10),
    );
    assert!(matches!(attempt, Err(IntentError::UnknownAgent(_) | IntentError::AgentForbidden(_))));
}

#[test]
fn creation_waits_for_the_owner_and_sends_both_limits_and_no_host() {
    let store = world().store;
    let kind = || IntentKind::AgentCreate { owner_id: AccountId::new("owner"), request: request() };
    let unconfirmed = decide(&store, kind()).expect("an unconfirmed create is asked for");
    assert!(matches!(unconfirmed.as_slice(), [Effect::RequireConfirmation { .. }]));

    let effects = handle_intent(&store, confirmed(kind()), Timestamp(10)).expect("create");
    let body = effects[0].body().expect("a call");
    assert_eq!(effects[0].method(), Some("agent.create"));
    assert_eq!(body["confirmed"], true);
    assert_eq!(body["role"], "named");
    assert_eq!(body["config"]["limits"]["max_concurrent_turns"], 2);
    assert_eq!(body["config"]["limits"]["max_cost_per_day_cents"], 500);
    assert_eq!(body["config"]["tools"], json!([]));
    assert!(body.get("host").is_none(), "a host binding is never sent");
}

#[test]
fn a_locked_owner_cannot_create_an_agent() {
    let store = world().store;
    let kind = IntentKind::AgentCreate { owner_id: AccountId::new("locked"), request: request() };
    assert_eq!(
        handle_intent(&store, confirmed(kind), Timestamp(10)).err(),
        Some(IntentError::AgentCustody(AgentCustodyProblem::Locked))
    );
}

#[test]
fn edits_need_a_known_live_agent_and_its_base() {
    let store = world().store;
    let edit =
        || IntentKind::AgentPersonaEdit { agent: "agent-1".into(), persona: request().persona };
    assert_eq!(
        decide(&store, edit()).err(),
        Some(IntentError::AgentRecordUnknown("agent-1".into()))
    );

    let store = with_agent(world(), record("agent-1", AgentLifecycle::Active, "owner")).store;
    let asked = decide(&store, edit()).expect("an unconfirmed edit is asked for");
    assert!(matches!(asked.as_slice(), [Effect::RequireConfirmation { .. }]));
    let effects = handle_intent(&store, confirmed(edit()), Timestamp(10)).expect("edit");
    assert_eq!(effects[0].method(), Some("agent.persona.edit"));
    assert_eq!(effects[0].body().expect("call")["agent_anchor"], "agent-1");

    let mut no_base = record("agent-1", AgentLifecycle::Active, "owner");
    no_base.base = serde_json::Value::Null;
    let store = with_agent(world(), no_base).store;
    let access =
        IntentKind::AgentAccessSet { agent: "agent-1".into(), access: AgentAccess::owner_only() };
    assert_eq!(
        handle_intent(&store, confirmed(access), Timestamp(10)).err(),
        Some(IntentError::AgentRecordBase("agent-1".into()))
    );
}

#[test]
fn retired_agents_are_read_only_and_an_allowlist_needs_humans() {
    let store = with_agent(world(), record("agent-1", AgentLifecycle::Retired, "owner")).store;
    assert_eq!(
        decide(&store, IntentKind::AgentDelete { agent: "agent-1".into() }).err(),
        Some(IntentError::AgentRecordRetired("agent-1".into()))
    );

    let store = with_agent(world(), record("agent-1", AgentLifecycle::Active, "owner")).store;
    let bad_access = AgentAccess {
        dm: AgentAccessMode::Owner,
        group_inviters: AgentAccessMode::Owner,
        allowed_humans: vec!["h-1".into()],
    };
    assert_eq!(
        handle_intent(
            &store,
            confirmed(IntentKind::AgentAccessSet { agent: "agent-1".into(), access: bad_access }),
            Timestamp(10),
        )
        .err(),
        Some(IntentError::AgentAccessInvalid)
    );
}

#[test]
fn runtime_install_needs_a_provider_and_a_key_that_never_serializes() {
    let store = world().store;
    let install = |key: Option<&str>| IntentKind::AgentRuntime {
        action: AgentRuntimeAction::Install,
        provider: "anthropic".into(),
        model: "model-x".into(),
        key: key.map(SecretInput::new),
        nonce: 1,
    };
    assert_eq!(
        handle_intent(&store, confirmed(install(None)), Timestamp(10)).err(),
        Some(IntentError::Empty("provider key"))
    );

    let asked = decide(&store, install(Some("sk-test"))).expect("install is asked for first");
    assert!(matches!(asked.as_slice(), [Effect::RequireConfirmation { .. }]));

    let effects =
        handle_intent(&store, confirmed(install(Some("sk-test"))), Timestamp(10)).expect("install");
    match &effects[0] {
        Effect::AgentRuntime { action, provider, key, .. } => {
            assert_eq!(*action, AgentRuntimeAction::Install);
            assert_eq!(provider, "anthropic");
            assert!(key.is_some(), "the key travels in the effect only");
        }
        other => panic!("test: expected a runtime effect, got {other:?}"),
    }
    let serialized = serde_json::to_string(&install(Some("sk-test"))).expect("serialize");
    assert!(!serialized.contains("sk-test"), "a key never serializes");
    assert!(!format!("{:?}", install(Some("sk-test"))).contains("sk-test"), "nor in debug output");
}

#[test]
fn jobs_pause_at_once_and_removal_is_asked_for() {
    let store = with_agent(world(), record("agent-1", AgentLifecycle::Active, "owner")).store;
    let job = |action| IntentKind::AgentJob {
        agent: "agent-1".into(),
        kind: AgentJobKind::Cron,
        id: "job-1".into(),
        action,
    };
    let paused = handle_intent(&store, Intent::human(job(AgentJobAction::Pause)), Timestamp(10))
        .expect("pause");
    assert_eq!(paused[0].method(), Some("agent.job.pause"));
    assert_eq!(paused[0].body().expect("call")["kind"], "cron");
    let removed = decide(&store, job(AgentJobAction::Remove)).expect("remove is asked for");
    assert!(matches!(removed.as_slice(), [Effect::RequireConfirmation { .. }]));
}

#[test]
fn the_jobs_of_an_agent_are_read_without_confirmation() {
    let store = with_agent(world(), record("agent-1", AgentLifecycle::Active, "owner")).store;
    let read = handle_intent(
        &store,
        Intent::human(IntentKind::AgentJobsRefresh { agent: "agent-1".into() }),
        Timestamp(10),
    )
    .expect("the job list is read");
    assert_eq!(read[0].method(), Some("agent.job.list"));
    assert_eq!(read[0].body().expect("call")["agent_anchor"], "agent-1");
    assert!(
        handle_intent(
            &store,
            Intent::human(IntentKind::AgentJobsRefresh { agent: " ".into() }),
            Timestamp(10)
        )
        .is_err(),
        "an empty agent is refused"
    );
}

/// The panel's follow step: the real rule ([`HostAction::follow`]) applied to
/// the stored status; an ended action is dropped, like the panel drops it.
fn follow(store: &Store, action: &mut Option<HostAction>) -> Option<std::path::PathBuf> {
    let step = action.as_ref()?.follow(store.agent_host());
    match step {
        HostStep::Waiting => None,
        HostStep::Ended { adopt } => {
            *action = None;
            adopt
        }
    }
}

#[test]
fn a_restart_adopts_the_runtime_once_and_never_early() {
    let mut feed = world();
    let answer = |nonce: u64| AgentHostStatus {
        state: AgentHostState::Running,
        prx_version: Some("0.1.0".into()),
        refusal: None,
        state_dir: Some("/state/prx".into()),
        applied: true,
        nonce,
    };
    let state_dir = std::path::PathBuf::from("/state/prx");
    let mut submitted = Some(HostAction::new(1, true));
    let mut restarts = Vec::new();

    // The install (action 1) answers running: one restart, with the state dir.
    let changes = feed.ok(1, DomainEvent::AgentHostUpdated(answer(1)));
    assert!(changes.agents);
    restarts.extend(follow(&feed.store, &mut submitted));
    assert_eq!(restarts, vec![state_dir.clone()]);

    // The restart (action 2) is submitted; until its answer, the stored
    // status (action 1) adopts nothing, so no restart runs early.
    submitted = Some(HostAction::new(2, false));
    assert_eq!(follow(&feed.store, &mut submitted), None);
    assert!(submitted.is_some(), "the wait for the restart's answer goes on");

    // Its answer is identical in every field but the number: still a change,
    // and the second restart runs with the same state dir.
    let changes = feed.ok(2, DomainEvent::AgentHostUpdated(answer(2)));
    assert!(changes.agents, "an identical answer is a change");
    restarts.extend(follow(&feed.store, &mut submitted));
    assert_eq!(restarts, vec![state_dir.clone(), state_dir]);

    // A later status read (action 3) that is refused adopts nothing.
    submitted = Some(HostAction::new(3, false));
    feed.ok(
        3,
        DomainEvent::AgentHostUpdated(AgentHostStatus {
            refusal: Some(AgentRefusal::PrxMissing),
            ..answer(3)
        }),
    );
    assert_eq!(follow(&feed.store, &mut submitted), None);
    assert!(submitted.is_none(), "the refused status ends the action");
    assert_eq!(restarts.len(), 2, "two restarts, the install's and the restart's");
}

fn listed(cursor: &str) -> DomainEvent {
    DomainEvent::AgentsListed { records: Vec::new(), journal_cursor: cursor.into() }
}

#[test]
fn a_background_failure_does_not_cancel_an_install() {
    let mut feed = world();
    let mut action = Some(HostAction::new(1, true));
    // A background watch fails: a failed effect of another method.
    feed.ok(
        6,
        DomainEvent::EffectFailed(EffectFailure {
            method: methods::AGENT_WATCH.to_owned(),
            code: FailureCode::Timeout,
            message: "timeout".into(),
        }),
    );
    assert_eq!(follow(&feed.store, &mut action), None);
    assert!(action.is_some_and(|a| a.installing()), "the install is still running");

    // Its own refusal (the bridge's status carrying its number) ends it,
    // keeps what was known of the runtime, and adopts nothing.
    feed.ok(
        7,
        DomainEvent::AgentHostUpdated(AgentHostStatus {
            state: AgentHostState::Running,
            prx_version: Some("0.1.0".into()),
            refusal: None,
            state_dir: Some("/state/prx".into()),
            applied: false,
            nonce: 0,
        }),
    );
    feed.ok(
        8,
        DomainEvent::AgentHostUpdated(AgentHostStatus {
            state: AgentHostState::Unknown,
            prx_version: None,
            refusal: Some(AgentRefusal::CredentialUnavailable),
            state_dir: None,
            applied: false,
            nonce: 1,
        }),
    );
    let host = feed.store.agent_host().cloned().expect("a status");
    assert_eq!(host.state, AgentHostState::Running, "the last seen state stays");
    assert_eq!(host.refusal, Some(AgentRefusal::CredentialUnavailable));
    assert_eq!(follow(&feed.store, &mut action), None);
    assert!(action.is_none(), "its own refusal ends the wait");
}

#[test]
fn a_declined_confirmation_clears_the_installing_state() {
    let mut action = HostAction::new(1, true);
    action.await_confirmation();
    assert!(!action.installing(), "waiting for the owner is not installing");
    assert_eq!(action.confirmation_closed(true), HostStep::Ended { adopt: None });

    // A confirmation lets the install run; a later close does not end it.
    let mut action = HostAction::new(2, true);
    action.await_confirmation();
    assert_eq!(action.confirmation_closed(false), HostStep::Waiting);
    assert!(action.installing());
    assert_eq!(action.confirmation_closed(true), HostStep::Waiting);
}

#[test]
fn a_listing_keeps_the_cursor_of_its_first_page_and_a_watch_advances_it() {
    let mut feed = world();
    feed.ok(10, listed("c-1"));
    assert_eq!(feed.store.agent_cursor(), Some("c-1"));
    // The later pages of the listing name no cursor.
    feed.ok(11, listed(""));
    feed.ok(12, listed(""));
    assert_eq!(feed.store.agent_cursor(), Some("c-1"));
    // A watch answer advances it.
    feed.ok(13, listed("c-2"));
    assert_eq!(feed.store.agent_cursor(), Some("c-2"));
}

#[test]
fn a_refused_cursor_is_dropped_and_no_watch_goes_out_until_a_listing() {
    let mut feed = world();
    feed.ok(10, listed("c-1"));
    feed.ok(11, DomainEvent::AgentCursorReset);
    assert_eq!(feed.store.agent_cursor(), None);
    let watch =
        handle_intent(&feed.store, Intent::human(IntentKind::AgentWatchRefresh), Timestamp(12));
    assert!(watch.is_err(), "no cursor, no watch: no retry storm");
    feed.ok(13, listed("c-3"));
    assert_eq!(feed.store.agent_cursor(), Some("c-3"));
}

#[test]
fn the_roster_and_the_creation_live_in_the_store() {
    let mut feed = world();
    assert_eq!(feed.store.agent_roster(), AgentRosterState::Loading);
    feed.ok(
        4,
        DomainEvent::AgentsListed {
            records: vec![record("agent-1", AgentLifecycle::Active, "owner")],
            journal_cursor: "c-9".into(),
        },
    );
    assert_eq!(feed.store.agent_roster(), AgentRosterState::Ready);
    assert_eq!(feed.store.agent("agent-1").map(|a| a.lifecycle), Some(AgentLifecycle::Active));
    assert_eq!(feed.store.agent_cursor(), Some("c-9"));
    feed.ok(
        5,
        DomainEvent::AgentCreationUpdated(AgentCreation {
            request_id: "req-1".into(),
            request_hash: "hash-1".into(),
            state: AgentCreationState::from_custody_state("authorization_pending"),
            agent_anchor: None,
            expires_at: Some(1_900_000_000),
        }),
    );
    let creation = feed.store.agent_creation("req-1").expect("the creation is kept");
    assert_eq!(creation.state, AgentCreationState::PendingPhone);
    assert!(!creation.state.is_terminal());
    assert!(AgentCreationState::from_custody_state("unknown_state").is_terminal());
}

#[test]
fn the_jobs_of_an_agent_the_owner_does_not_hold_are_not_touched() {
    let store = world().store;
    let refresh = IntentKind::AgentJobsRefresh { agent: "agent-1".into() };
    assert!(matches!(decide(&store, refresh), Err(IntentError::AgentRecordUnknown(_))));
    let job = IntentKind::AgentJob {
        agent: "agent-1".into(),
        kind: AgentJobKind::Cron,
        id: "job-1".into(),
        action: AgentJobAction::Pause,
    };
    assert!(matches!(decide(&store, job), Err(IntentError::AgentRecordUnknown(_))));
    let retired = with_agent(world(), record("agent-1", AgentLifecycle::Retired, "owner")).store;
    let refresh = IntentKind::AgentJobsRefresh { agent: "agent-1".into() };
    assert!(matches!(decide(&retired, refresh), Err(IntentError::AgentRecordRetired(_))));
}

fn approval(id: &str, agent: &str, expires_at: Option<i64>) -> AgentApproval {
    AgentApproval {
        approval_id: id.into(),
        conversation_id: "conv-1".into(),
        agent: agent.into(),
        tool: "shell".into(),
        preview: "ls".into(),
        status: "pending".into(),
        expires_at,
    }
}

fn decide_approval(owner: &str, id: &str) -> IntentKind {
    IntentKind::AgentApprovalDecide {
        owner_id: AccountId::new(owner),
        approval_id: id.into(),
        approved: true,
    }
}

fn with_approvals(feed: Feed, approvals: Vec<AgentApproval>) -> Store {
    let mut feed = with_agent(feed, record("agent-1", AgentLifecycle::Active, "owner"));
    feed.ok(5, DomainEvent::AgentApprovalsUpdated(approvals));
    feed.store
}

#[test]
fn an_approval_is_decided_by_the_owner_of_the_agent_it_names() {
    let store = with_approvals(world(), vec![approval("ap-1", "agent-1", None)]);
    let sent = decide(&store, decide_approval("owner", "ap-1")).expect("the owner decides");
    assert_eq!(sent[0].method(), Some("agent.approval.decide"));
    assert_eq!(sent[0].body().expect("call")["decision"], "approved");
    // The runtime name PRX uses names the same agent.
    let store = with_approvals(world(), vec![approval("ap-1", "a_agent-1", None)]);
    assert!(decide(&store, decide_approval("owner", "ap-1")).is_ok());
}

#[test]
fn an_approval_is_refused_for_another_account_an_unknown_id_or_an_expired_one() {
    let mut feed = world();
    feed.ok(
        6,
        DomainEvent::AccountUpdated(account(
            "other",
            AccountKind::HumanInteractive,
            LockState::Unlocked,
            Networking::Live,
        )),
    );
    let store = with_approvals(
        feed,
        vec![approval("ap-1", "agent-1", None), approval("ap-old", "agent-1", Some(0))],
    );
    assert!(matches!(
        decide(&store, decide_approval("other", "ap-1")),
        Err(IntentError::AgentApprovalOwner(_))
    ));
    assert!(matches!(
        decide(&store, decide_approval("owner", "ap-none")),
        Err(IntentError::AgentApprovalUnknown(_))
    ));
    assert!(matches!(
        decide(&store, decide_approval("owner", "ap-old")),
        Err(IntentError::AgentApprovalExpired(_))
    ));
    assert!(matches!(
        decide(&store, decide_approval("locked", "ap-1")),
        Err(IntentError::AgentCustody(_))
    ));
    assert!(matches!(
        decide(&store, decide_approval("owner", " ")),
        Err(IntentError::Empty("approval id"))
    ));
}

#[test]
fn an_approval_expires_by_its_unix_seconds() {
    let at = approval("ap", "agent-1", Some(100));
    assert!(!at.is_expired(99_999));
    assert!(at.is_expired(100_000));
    assert!(!approval("ap", "agent-1", None).is_expired(i64::MAX));
}
