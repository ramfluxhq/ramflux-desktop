// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! AI participants an owner custodies (DESK-AGENT-01) in policy and in the
//! store: creation only for an unlocked, connected person's account and
//! only when the daemon offers it; the confirmation takes exactly three
//! words and no passphrase; the export only for the custodian, to an
//! absolute location, never for a person's account; custodian links and
//! export outcomes live in the store and carry no credential.

mod common;

use common::*;
use ramflux_desktop_core::*;

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
    let person = |id, lock, net| {
        DomainEvent::AccountUpdated(account(id, AccountKind::HumanInteractive, lock, net))
    };
    feed.ok(2, person("owner", LockState::Unlocked, Networking::Live));
    feed.ok(3, person("locked", LockState::Locked, Networking::Live));
    feed.ok(4, person("pending", LockState::Unlocked, Networking::Pending));
    feed.ok(
        5,
        DomainEvent::AccountUpdated(account(
            "bot",
            AccountKind::HeadlessAi,
            LockState::Unlocked,
            Networking::Live,
        )),
    );
    feed.ok(
        6,
        DomainEvent::CustodianKnown {
            agent: AccountId::new("bot"),
            custodian: AccountId::new("owner"),
        },
    );
    feed
}

fn decide(store: &Store, kind: IntentKind) -> Result<Vec<Effect>, IntentError> {
    handle_intent(store, Intent::human(kind), Timestamp(10))
}

fn begin(custodian: &str) -> IntentKind {
    IntentKind::BeginAgentCeremony {
        label: " Ledger ".into(),
        custodian_id: AccountId::new(custodian),
    }
}

fn export(owner: &str, agent: &str, path: &str) -> IntentKind {
    IntentKind::ExportAgentCredentials {
        owner_id: AccountId::new(owner),
        account_id: AccountId::new(agent),
        path: path.into(),
    }
}

#[test]
fn creation_begins_as_the_owner_with_the_kind_and_custodian_named() {
    let feed = world();
    let effects = decide(&feed.store, begin("owner")).expect("allowed");
    assert_eq!(effects[0].method(), Some(methods::CUSTODY_BEGIN));
    let body = effects[0].body().expect("body");
    assert_eq!(body["display_name"], "Ledger");
    assert_eq!(body["principal_kind"], "headless_ai");
    assert_eq!(body["custodian_account"], "owner");
}

#[test]
fn creation_is_refused_for_every_account_the_daemon_would_refuse() {
    let feed = world();
    let problem = |custodian: &str| decide(&feed.store, begin(custodian)).err();
    assert_eq!(problem("nobody"), Some(IntentError::AgentCustody(AgentCustodyProblem::Unknown)));
    assert_eq!(problem("locked"), Some(IntentError::AgentCustody(AgentCustodyProblem::Locked)));
    assert_eq!(
        problem("pending"),
        Some(IntentError::AgentCustody(AgentCustodyProblem::NotConnected))
    );
    assert_eq!(problem("bot"), Some(IntentError::AgentCustody(AgentCustodyProblem::NotPerson)));
    // Not offered, an empty name, a ceremony already running, and agents.
    let mut bare = Feed::new();
    bare.ok(
        2,
        DomainEvent::AccountUpdated(account(
            "owner",
            AccountKind::HumanInteractive,
            LockState::Unlocked,
            Networking::Live,
        )),
    );
    assert_eq!(decide(&bare.store, begin("owner")).err(), Some(IntentError::CeremonyNotOffered));
    let blank = IntentKind::BeginAgentCeremony {
        label: "  ".into(),
        custodian_id: AccountId::new("owner"),
    };
    assert!(matches!(decide(&feed.store, blank), Err(IntentError::Empty(_))));
    let mut running = world();
    running.ok(
        7,
        DomainEvent::CeremonyUpdated(Ceremony {
            phase: CeremonyPhase::Prepared,
            ceremony_id: "c-1".into(),
            label: "Ledger".into(),
            expires_at: Timestamp(300_000),
        }),
    );
    assert_eq!(decide(&running.store, begin("owner")).err(), Some(IntentError::CeremonyPhase));
    let by_agent = Intent::agent(ParticipantId::new("p-bot"), begin("owner"));
    assert!(handle_intent(&feed.store, by_agent, Timestamp(10)).is_err(), "human only");
}

#[test]
fn the_confirmation_takes_three_words_once_asked_and_no_passphrase() {
    let mut feed = world();
    let confirm = |count: usize| IntentKind::ConfirmAgentCeremony {
        words: (0..count).map(|_| SecretInput::new("abandon")).collect(),
    };
    assert_eq!(decide(&feed.store, confirm(3)).err(), Some(IntentError::CeremonyPhase));
    for (at, phase) in [
        (7, CeremonyPhase::Prepared),
        (8, CeremonyPhase::PhraseReady),
        (9, CeremonyPhase::Challenged { positions: [3, 11, 20] }),
    ] {
        feed.ok(
            at,
            DomainEvent::CeremonyUpdated(Ceremony {
                phase,
                ceremony_id: "c-1".into(),
                label: "Ledger".into(),
                expires_at: Timestamp(300_000),
            }),
        );
    }
    for count in [0, 2, 4] {
        assert_eq!(decide(&feed.store, confirm(count)).err(), Some(IntentError::CeremonyWords));
    }
    let effects = decide(&feed.store, confirm(3)).expect("allowed");
    let Effect::SecretCall { method, secrets, .. } = &effects[0] else { panic!("secret call") };
    assert_eq!(method, methods::CUSTODY_CONFIRM);
    assert_eq!(secrets.len(), 3, "the words only: no passphrase travels");
}

#[test]
fn the_export_goes_to_the_custodian_alone_and_names_nothing_secret() {
    let feed = world();
    let effects = decide(&feed.store, export("owner", "bot", " /Users/a/bot.bundle ")).expect("ok");
    assert_eq!(effects[0].method(), Some(methods::AGENT_EXPORT_CREDENTIALS));
    let body = effects[0].body().expect("body");
    assert_eq!(body["path"], "/Users/a/bot.bundle");
    assert_eq!(body["account"], "bot");
    assert_eq!(body["owner_id"], "owner");
    let refused = |owner: &str, agent: &str, path: &str| {
        decide(&feed.store, export(owner, agent, path)).err()
    };
    let no = |r| Some(IntentError::AgentExport(r));
    assert_eq!(refused("owner", "bot", ""), no(AgentExportRefusal::NoLocation));
    assert_eq!(refused("owner", "bot", "relative/bot.bundle"), no(AgentExportRefusal::NoLocation));
    assert_eq!(refused("nobody", "bot", "/x"), no(AgentExportRefusal::NotOwnerSession));
    assert_eq!(refused("locked", "bot", "/x"), no(AgentExportRefusal::OwnerLocked));
    assert_eq!(refused("pending", "bot", "/x"), no(AgentExportRefusal::NotConnected));
    assert_eq!(refused("bot", "bot", "/x"), no(AgentExportRefusal::NotCustodian));
    assert_eq!(refused("owner", "owner", "/x"), no(AgentExportRefusal::NotCustodian));
    assert_eq!(refused("owner", "nobody", "/x"), no(AgentExportRefusal::NotCustodian));
}

#[test]
fn another_persons_account_cannot_export_what_it_does_not_custody() {
    let mut feed = world();
    feed.ok(
        7,
        DomainEvent::AccountUpdated(account(
            "other",
            AccountKind::HumanInteractive,
            LockState::Unlocked,
            Networking::Live,
        )),
    );
    assert_eq!(
        decide(&feed.store, export("other", "bot", "/x")).err(),
        Some(IntentError::AgentExport(AgentExportRefusal::NotCustodian))
    );
}

#[test]
fn links_and_outcomes_live_in_the_store_and_say_what_changed() {
    let mut feed = world();
    assert_eq!(feed.store.custodian_of(&AccountId::new("bot")), Some(&AccountId::new("owner")));
    let same = feed.ok(
        8,
        DomainEvent::CustodianKnown {
            agent: AccountId::new("bot"),
            custodian: AccountId::new("owner"),
        },
    );
    assert!(same.is_empty(), "a link told again changes nothing");
    let done = AgentExport {
        account_id: AccountId::new("bot"),
        path: "/Users/a/bot.bundle".into(),
        phase: AgentExportPhase::Exported,
    };
    let changes = feed.ok(9, DomainEvent::AgentExportUpdated(done.clone()));
    assert!(changes.agent_exports.contains(&AccountId::new("bot")));
    assert_eq!(feed.store.agent_export(&AccountId::new("bot")), Some(&done));
    assert!(feed.ok(10, DomainEvent::AgentExportUpdated(done)).is_empty());
    let refused = AgentExport {
        account_id: AccountId::new("bot"),
        path: "/Users/a/bot.bundle".into(),
        phase: AgentExportPhase::Refused(AgentExportRefusal::CredentialsGone),
    };
    feed.ok(11, DomainEvent::AgentExportUpdated(refused.clone()));
    assert_eq!(feed.store.agent_export(&AccountId::new("bot")), Some(&refused));
}

#[test]
fn the_new_events_and_records_survive_serialization() {
    let events = [
        DomainEvent::CustodianKnown {
            agent: AccountId::new("bot"),
            custodian: AccountId::new("owner"),
        },
        DomainEvent::AgentExportUpdated(AgentExport {
            account_id: AccountId::new("bot"),
            path: "/x".into(),
            phase: AgentExportPhase::Refused(AgentExportRefusal::FileExists),
        }),
        DomainEvent::CeremonyUpdated(Ceremony {
            phase: CeremonyPhase::Failed(CeremonyFailure::CustodianRefused),
            ceremony_id: String::new(),
            label: "Ledger".into(),
            expires_at: Timestamp(0),
        }),
    ];
    for event in events {
        let text = serde_json::to_string(&event).expect("json");
        let back: DomainEvent = serde_json::from_str(&text).expect("back");
        assert_eq!(back, event);
    }
    let kind = begin("owner");
    let text = serde_json::to_string(&kind).expect("json");
    assert!(!text.contains("passphrase") && !text.contains("words"));
}
