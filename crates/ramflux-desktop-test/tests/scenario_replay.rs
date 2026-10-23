// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! End-to-end replay of every bundled scenario through the store, with one
//! scenario-specific end-state assertion each.

use ramflux_desktop_core::selectors::{
    attention, context_stack, in_flight, participants, timeline,
};
use ramflux_desktop_core::*;
use ramflux_desktop_test::{
    DEFAULT_EPOCH, SCENARIO_NAMES, ScenarioPlayer, fixtures_dir, load_fixture,
};

const DEFAULT_EPOCH_MS: i64 = DEFAULT_EPOCH.millis();

fn player(name: &str) -> ScenarioPlayer {
    let scenario = load_fixture(name).unwrap_or_else(|e| panic!("{name}: {e}"));
    ScenarioPlayer::new(&scenario)
}

fn assert_clean(player: &ScenarioPlayer) {
    assert!(player.errors().is_empty(), "apply errors: {:?}", player.errors());
    assert!(player.store().rejections().is_empty());
    assert!(player.applied() > 0);
}

fn lifecycle(player: &ScenarioPlayer, id: &str) -> Lifecycle {
    player.store().moment(&MomentId::new(id)).unwrap_or_else(|| panic!("moment {id}")).lifecycle
}

fn attention_ids(player: &ScenarioPlayer) -> Vec<String> {
    attention(player.store()).into_iter().map(|i| i.moment_id.to_string()).collect()
}

fn approve(grant: &str, confirmed: bool) -> Intent {
    held_if(Intent::human(IntentKind::Approve { grant_id: GrantId::new(grant) }), confirmed)
}

/// Adds the hold-to-confirm evidence the strip would add.
fn held_if(intent: Intent, confirmed: bool) -> Intent {
    if confirmed { intent.confirmed_by_human(HumanConfirmation::hold(1_200)) } else { intent }
}

#[test]
fn every_fixture_parses_and_is_listed() {
    let mut on_disk: Vec<String> = std::fs::read_dir(fixtures_dir())
        .expect("fixtures dir")
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            e.file_name().to_str().and_then(|n| n.strip_suffix(".ron")).map(str::to_owned)
        })
        .collect();
    let mut expected: Vec<String> = SCENARIO_NAMES.iter().map(|n| (*n).to_owned()).collect();
    for name in SCENARIO_NAMES {
        let scenario = load_fixture(name).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(scenario.name, name);
        assert!(!scenario.timeline.is_empty());
        assert!(!scenario.description.is_empty());
        for companion in &scenario.companions {
            let other = load_fixture(&companion.scenario).expect("companion fixture");
            assert!(other.companions.is_empty(), "companions do not nest");
            expected.push(companion.scenario.clone());
        }
    }
    on_disk.sort();
    expected.sort();
    assert_eq!(on_disk, expected, "every fixture is listed or a companion");
}

#[test]
fn every_timeline_replays_without_errors() {
    let companions = SCENARIO_NAMES
        .iter()
        .flat_map(|n| load_fixture(n).expect("fixture").companions)
        .map(|c| c.scenario);
    let names: Vec<String> =
        SCENARIO_NAMES.iter().map(|n| (*n).to_owned()).chain(companions).collect();
    for name in &names {
        let name = name.as_str();
        let mut p = player(name);
        p.run_to_end();
        assert!(p.errors().is_empty(), "{name}: {:?}", p.errors());
    }
}

#[test]
fn s01_tofu_node_needs_a_hold_then_create_lock_and_unlock() {
    let mut p = player("s01_first_launch");
    p.advance(1000);
    assert_eq!(p.store().source_status(), SourceStatus::Disconnected);
    p.advance(2000);
    let harbor = p.store().node(&NodeId::new("n-harbor")).expect("invite node");
    assert!(!harbor.confirmed, "an invite node waits for confirmation");

    let add = IntentKind::AddNode { url: "https://studio.example.org".into() };
    assert!(p.dispatch(Intent::human(add)).expect("add").is_empty());
    p.advance(400);
    let studio = p.store().node(&NodeId::new("n-studio")).expect("discovered node");
    assert!(!studio.confirmed && studio.pin_source.is_trust_on_first_use());

    let confirm = || IntentKind::ConfirmNode {
        node_id: NodeId::new("n-studio"),
        pin_source: PinSource::Manual,
    };
    let pending = p.dispatch(Intent::human(confirm())).expect("confirm");
    assert!(matches!(pending.as_slice(), [Effect::RequireConfirmation { .. }]));
    assert!(p.dispatch(held_if(Intent::human(confirm()), true)).expect("held").is_empty());
    p.advance(200);
    assert!(p.store().node(&NodeId::new("n-studio")).expect("node").confirmed);
    assert!(
        p.store().audit().iter().any(|a| a.action == methods::NODE_CONFIRM && a.human_confirmed)
    );

    let create = IntentKind::CreateAccount {
        kind: AccountKind::HumanInteractive,
        label: "Personal".into(),
        node_id: NodeId::new("n-studio"),
        secret: SecretInput::new("river stone lantern"),
    };
    assert!(p.dispatch(Intent::human(create)).expect("create").is_empty());
    p.advance(500);
    let account = p.store().account(&AccountId::new("a-main")).expect("account");
    assert_eq!(account.lock_state, LockState::Unlocked);
    assert_eq!(account.node_id, Some(NodeId::new("n-studio")));
    assert_eq!(context_stack(p.store()), [ContextId::new("c-notes")]);

    let lock = IntentKind::LockAccount { account_id: AccountId::new("a-main") };
    assert!(p.dispatch(Intent::human(lock)).expect("lock").is_empty());
    p.advance(100);
    let account = p.store().account(&AccountId::new("a-main")).expect("account");
    assert_eq!(account.lock_state, LockState::Locked, "the local echo locks the account");
    let unlock = IntentKind::Unlock {
        account_id: AccountId::new("a-main"),
        secret: SecretInput::new("river stone lantern"),
    };
    assert!(p.dispatch(Intent::human(unlock)).expect("unlock").is_empty());
    p.advance(200);

    assert_clean(&p);
    let account = p.store().account(&AccountId::new("a-main")).expect("account");
    assert_eq!(account.lock_state, LockState::Unlocked);
    let labels: Vec<&str> = p.source().effects().iter().map(|e| e.label.as_str()).collect();
    assert_eq!(
        labels,
        [
            methods::NODE_ADD,
            methods::NODE_CONFIRM,
            methods::ACCOUNT_CREATE,
            methods::ACCOUNT_LOCK,
            methods::ACCOUNT_UNLOCK
        ]
    );
}

#[test]
fn s02_streams_reply_completes_transfer_and_sends() {
    let mut p = player("s02_daily");
    p.advance(3000);
    let send = IntentKind::SendMessage {
        context_id: ContextId::new("c-lena"),
        text: "12:30 works".into(),
    };
    assert!(p.dispatch(Intent::human(send)).expect("send").is_empty());
    p.advance(100);

    assert_clean(&p);
    let store = p.store();
    assert_eq!(store.contexts().count(), 3);
    assert_eq!(store.context(&ContextId::new("c-lena")).expect("c-lena").unread, 2);
    let lena = timeline(store, &ContextId::new("c-lena"));
    assert_eq!(lena.len(), 5);
    assert!(lena[0].at < Timestamp(DEFAULT_EPOCH_MS), "history sorts before today");
    let reply = store.moment(&MomentId::new("m-orbit-a")).expect("reply");
    let MomentKind::AgentSurface(surface) = &reply.kind else { panic!("not a surface") };
    assert_eq!(
        surface.blocks[0].data["text"],
        "Three decisions were made: keep the open layout, move the kitchen east, and defer lighting."
    );
    assert!(!surface.streaming);
    assert_eq!(reply.lifecycle, Lifecycle::Done);
    let object = store.object(&ObjectId::new("o-floorplan")).expect("object");
    assert_eq!(object.transfer, TransferState::Complete);
    assert!(in_flight(store).is_empty());
    assert_eq!(context_stack(store), [ContextId::new("c-team")]);
    p.advance(2000);
    let reply = p.store().moment(&MomentId::new("m-lena-reply")).expect("reply");
    assert!(matches!(&reply.kind, MomentKind::Message(m) if m.receipt == Receipt::Read));
}

#[test]
fn s03_confirmed_approval_resolves_grant_and_closes_intervention() {
    let mut p = player("s03_intervention");
    p.advance(1000);
    assert_eq!(attention_ids(&p), ["m-approve"]);

    let pending = p.dispatch(approve("g-rotate", false)).expect("approve");
    assert!(matches!(pending.as_slice(), [Effect::RequireConfirmation { .. }]));
    assert!(p.source().effects().is_empty(), "unconfirmed approval must not reach the source");
    p.advance(1000);
    assert_eq!(lifecycle(&p, "m-approve"), Lifecycle::NeedsMe);

    assert!(p.dispatch(approve("g-rotate", true)).expect("approve").is_empty());
    p.advance(1000);

    assert_clean(&p);
    let grant = p.store().grant(&GrantId::new("g-rotate")).expect("grant");
    assert_eq!(grant.state, GrantState::Approved);
    assert_eq!(lifecycle(&p, "m-approve"), Lifecycle::Done);
    assert_eq!(lifecycle(&p, "m-result"), Lifecycle::Done);
    assert!(attention_ids(&p).is_empty());
}

#[test]
fn s03_approval_is_refused_after_the_grant_expires() {
    let mut p = player("s03_intervention");
    // The fixture grant expires ten minutes after the scenario epoch.
    p.advance(600_000);
    assert_eq!(
        p.dispatch(approve("g-rotate", true)).err(),
        Some(IntentError::GrantExpired(GrantId::new("g-rotate")))
    );
    assert!(p.source().effects().is_empty());
}

#[test]
fn s04_key_change_is_sticky_until_verified() {
    let mut p = player("s04_safety");
    p.advance(1000);
    let mira = ParticipantId::new("p-mira");
    assert_eq!(p.store().participant(&mira).expect("mira").safety, SafetyState::KeyChanged);
    let reasons: Vec<_> = attention(p.store()).into_iter().map(|a| a.reason).collect();
    assert_eq!(reasons, [AttentionReason::ContactRequest, AttentionReason::Safety]);
    p.advance(5000);
    assert_eq!(lifecycle(&p, "m-key"), Lifecycle::NeedsMe, "warning must stay until verified");

    let verify = IntentKind::VerifySafetyNumber { participant_id: mira.clone() };
    assert!(p.dispatch(Intent::human(verify)).expect("verify").is_empty());
    p.advance(200);

    assert_clean(&p);
    assert_eq!(p.store().participant(&mira).expect("mira").safety, SafetyState::Verified);
    assert_eq!(lifecycle(&p, "m-key"), Lifecycle::Done);
    assert_eq!(lifecycle(&p, "m-verified"), Lifecycle::Done);
    assert_eq!(attention_ids(&p), ["m-noor"], "only the contact request is left");
    let accept = IntentKind::AcceptContact { moment_id: MomentId::new("m-noor") };
    assert!(p.dispatch(Intent::human(accept)).expect("accept").is_empty());
    p.advance(200);
    assert!(attention_ids(&p).is_empty());
    assert!(p.store().participant(&ParticipantId::new("p-noor")).is_some_and(|n| n.contact));
}

fn garden(op: GovernanceOp, confirmed: bool) -> Intent {
    held_if(
        Intent::human(IntentKind::Governance { context_id: ContextId::new("c-garden"), op }),
        confirmed,
    )
}

fn members(p: &ScenarioPlayer) -> Vec<String> {
    participants(p.store(), &ContextId::new("c-garden")).iter().map(|m| m.id.to_string()).collect()
}

#[test]
fn s05_governance_runs_through_intents_and_confirmation() {
    let mut p = player("s05_group_governance");
    p.advance(500);
    assert_eq!(members(&p), ["p-me", "p-lena", "p-tomas"]);

    let add = GovernanceOp::AddMember { participant_id: "p-mira".into(), role: Role::Member };
    assert!(p.dispatch(garden(add, false)).expect("add").is_empty(), "adding is low risk");
    p.advance(1500);
    assert_eq!(members(&p), ["p-me", "p-lena", "p-tomas", "p-mira"]);

    let promote = GovernanceOp::SetRole { participant_id: "p-lena".into(), role: Role::Admin };
    let pending = p.dispatch(garden(promote.clone(), false)).expect("set role");
    assert!(matches!(pending.as_slice(), [Effect::RequireConfirmation { .. }]));
    let kick = GovernanceOp::Kick { participant_id: "p-tomas".into() };
    let pending = p.dispatch(garden(kick.clone(), false)).expect("kick");
    assert!(matches!(pending.as_slice(), [Effect::RequireConfirmation { .. }]));
    let calls = p.source().effects().len();
    p.advance(500);
    assert_eq!(members(&p), ["p-me", "p-lena", "p-tomas", "p-mira"], "nothing applied unconfirmed");
    assert_eq!(p.source().effects().len(), calls);

    assert!(p.dispatch(garden(promote, true)).expect("set role").is_empty());
    assert!(p.dispatch(garden(kick, true)).expect("kick").is_empty());
    p.advance(500);

    assert_clean(&p);
    assert_eq!(members(&p), ["p-me", "p-lena", "p-mira"]);
    let role = p.store().moment(&MomentId::new("m-role")).expect("role moment");
    assert!(matches!(&role.kind, MomentKind::Governance(g) if g.role == Some(Role::Admin)));
    assert!(p.store().moment(&MomentId::new("m-removed")).is_some());
}

#[test]
fn s06_task_passes_through_needs_me_before_done() {
    let mut p = player("s06_in_progress");
    p.advance(200);
    let start = IntentKind::StartTask {
        context_id: ContextId::new("c-trip"),
        request: "Book a train to Lyon on Friday morning".into(),
    };
    assert!(p.dispatch(Intent::human(start)).expect("start").is_empty());
    p.advance(1000);
    assert_eq!(lifecycle(&p, "m-task"), Lifecycle::InProgress);
    p.advance(2000);

    assert_eq!(lifecycle(&p, "m-task"), Lifecycle::NeedsMe);
    assert!(attention_ids(&p).contains(&"m-task".to_owned()));
    assert!(in_flight(p.store()).contains(&MomentId::new("m-task")));
    let task = p.store().task(&TaskId::new("t-trip")).expect("task");
    assert_eq!(task.steps[2].lifecycle, Lifecycle::NeedsMe);

    // Medium risk: no confirmation bar, the call goes straight to the source.
    assert!(p.dispatch(approve("g-hold", false)).expect("approve").is_empty());
    p.advance(2000);

    assert_clean(&p);
    let task = p.store().task(&TaskId::new("t-trip")).expect("task");
    assert_eq!(task.lifecycle, Lifecycle::Done);
    assert!(task.steps.iter().all(|s| s.lifecycle == Lifecycle::Done));
    assert_eq!(lifecycle(&p, "m-task"), Lifecycle::Done);
    assert!(in_flight(p.store()).is_empty());
    assert!(attention_ids(&p).is_empty());
}

#[test]
fn s07_conflict_reveal_and_handoff_complete() {
    let mut p = player("s07_conflict_privacy_handoff");
    p.advance(500);
    assert_eq!(attention_ids(&p), ["m-conflict", "m-handoff"]);
    let declined_elsewhere = {
        let mut other = player("s07_conflict_privacy_handoff");
        other.advance(500);
        let decline = IntentKind::DeclineHandoff { moment_id: MomentId::new("m-handoff") };
        assert!(other.dispatch(Intent::human(decline)).expect("decline").is_empty());
        other.advance(200);
        lifecycle(&other, "m-handoff")
    };
    assert_eq!(declined_elsewhere, Lifecycle::Dismissed);
    let resolve = IntentKind::ResolveConflict {
        moment_id: MomentId::new("m-conflict"),
        resolution: ConflictResolution::KeepBoth,
    };
    assert!(p.dispatch(Intent::human(resolve)).expect("resolve").is_empty());
    let reveal = || IntentKind::Reveal { moment_id: MomentId::new("m-iban") };
    let pending = p.dispatch(Intent::human(reveal())).expect("reveal");
    assert!(
        matches!(pending.as_slice(), [Effect::RequireConfirmation { .. }]),
        "a reveal needs a hold"
    );
    assert!(p.dispatch(held_if(Intent::human(reveal()), true)).expect("reveal").is_empty());
    p.advance(6000);
    assert_eq!(lifecycle(&p, "m-handoff"), Lifecycle::NeedsMe, "handoff waits for the user");
    let accept = IntentKind::AcceptHandoff { moment_id: MomentId::new("m-handoff") };
    assert!(p.dispatch(Intent::human(accept)).expect("accept").is_empty());
    p.advance(200);

    assert_clean(&p);
    let store = p.store();
    let conflict = store.moment(&MomentId::new("m-conflict")).expect("conflict");
    let MomentKind::Conflict(payload) = &conflict.kind else { panic!("not a conflict") };
    assert_eq!(payload.resolution, Some(ConflictResolution::KeepBoth));
    let iban = store.moment(&MomentId::new("m-iban")).expect("reveal");
    let MomentKind::PrivacyReveal(payload) = &iban.kind else { panic!("not a reveal") };
    assert_eq!(payload.revealed.as_ref().map(|r| r.by.as_str()), Some("p-me"));
    assert!(payload.revealed.as_ref().is_some_and(|r| r.at > Timestamp(0)), "stamped on delivery");
    assert!(payload.content.is_some());
    assert!(
        store.audit().iter().any(|a| a.action == methods::PRIVACY_REVEAL && a.subject == "m-iban"),
        "the held reveal is audited"
    );
    let handoff = store.moment(&MomentId::new("m-handoff")).expect("handoff");
    let MomentKind::Handoff(payload) = &handoff.kind else { panic!("not a handoff") };
    assert_eq!(payload.to_device, Some(ParticipantId::new("d-laptop")));
    assert!(attention_ids(&p).is_empty());
}

#[test]
fn s08_failures_degrade_and_unknown_block_falls_back() {
    let mut p = player("s08_failures");
    p.advance(4000);

    assert_clean(&p);
    let store = p.store();
    assert_eq!(store.source_status(), SourceStatus::Disconnected);
    assert_eq!(store.node(&NodeId::new("n-harbor")).expect("node").health, NodeHealth::Unreachable);
    let object = store.object(&ObjectId::new("o-photos")).expect("object");
    assert!(matches!(object.transfer, TransferState::Failed { .. }));
    assert_eq!(lifecycle(&p, "m-upload"), Lifecycle::Failed);
    let chart = store.moment(&MomentId::new("m-chart")).expect("chart");
    let MomentKind::AgentSurface(surface) = &chart.kind else { panic!("not a surface") };
    assert_eq!(surface.blocks[0].kind, BlockKind::Heading);
    assert_eq!(surface.blocks[1].kind, BlockKind::Fallback);
    assert_eq!(surface.blocks[1].data["series"][1], 98);
}

#[test]
fn s08_retry_starts_a_new_transfer_that_completes() {
    let mut p = player("s08_failures");
    p.advance(4000);
    let retry = IntentKind::RetryTransfer { object_id: ObjectId::new("o-photos") };
    assert!(p.dispatch(Intent::human(retry)).expect("retry").is_empty());
    p.advance(1500);
    assert_clean(&p);
    let object = p.store().object(&ObjectId::new("o-photos")).expect("object");
    assert_eq!(object.transfer, TransferState::Complete);
    assert_eq!(lifecycle(&p, "m-upload-retry"), Lifecycle::Done);
}

#[test]
fn s09_and_its_companion_replay_as_separate_accounts() {
    let mut personal = player("s09_multi_account_day");
    personal.advance(2000);
    assert_clean(&personal);
    assert_eq!(attention_ids(&personal), ["m-ines"]);
    let mut work = player("s09_work_account");
    work.advance(2000);
    assert_eq!(attention_ids(&work), ["m-deploy"]);
    assert!(personal.store().context(&ContextId::new("c-release")).is_none(), "stores stay apart");

    let approve_deploy = approve("g-deploy", false);
    assert!(work.dispatch(approve_deploy).expect("medium risk").is_empty());
    work.advance(800);
    assert_clean(&work);
    assert!(attention_ids(&work).is_empty());
    assert_eq!(lifecycle(&work, "m-deployed"), Lifecycle::Done);
}
