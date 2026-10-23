// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! End-to-end replay of every bundled scenario through the store, with one
//! scenario-specific end-state assertion each.

use ramflux_desktop_core::selectors::{
    attention, context_stack, in_flight, participants, timeline,
};
use ramflux_desktop_core::*;
use ramflux_desktop_test::{SCENARIO_NAMES, ScenarioPlayer, fixtures_dir, load_fixture};

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
    Intent::human(IntentKind::Approve { grant_id: GrantId::new(grant), confirmed })
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
    on_disk.sort();
    assert_eq!(on_disk, SCENARIO_NAMES);
    for name in SCENARIO_NAMES {
        let scenario = load_fixture(name).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(scenario.name, name);
        assert!(!scenario.timeline.is_empty());
        assert!(!scenario.description.is_empty());
    }
}

#[test]
fn every_timeline_replays_without_errors() {
    for name in SCENARIO_NAMES {
        let mut p = player(name);
        p.run_to_end();
        assert!(p.errors().is_empty(), "{name}: {:?}", p.errors());
    }
}

#[test]
fn s01_unlock_enters_workspace() {
    let mut p = player("s01_first_launch");
    p.advance(1000);
    assert_eq!(p.store().source_status(), SourceStatus::Disconnected);
    p.advance(4000);
    assert_eq!(p.store().source_status(), SourceStatus::Locked);
    let account = p.store().account(&AccountId::new("a-main")).expect("account");
    assert_eq!(account.lock_state, LockState::Locked);

    let unlock = IntentKind::Unlock {
        account_id: AccountId::new("a-main"),
        secret: SecretInput::new("river stone lantern"),
    };
    assert!(p.dispatch(Intent::human(unlock)).expect("unlock").is_empty());
    p.advance(1000);

    assert_clean(&p);
    assert_eq!(p.store().source_status(), SourceStatus::Connected);
    let account = p.store().account(&AccountId::new("a-main")).expect("account");
    assert_eq!(account.lock_state, LockState::Unlocked);
    assert_eq!(context_stack(p.store()), [ContextId::new("c-notes")]);
    assert_eq!(p.source().effects()[0].label, "Unlock");
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
    assert_eq!(timeline(store, &ContextId::new("c-lena")).len(), 3);
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
fn s04_key_change_is_sticky_until_verified() {
    let mut p = player("s04_safety");
    p.advance(1000);
    let mira = ParticipantId::new("p-mira");
    assert_eq!(p.store().participant(&mira).expect("mira").safety, SafetyState::KeyChanged);
    assert_eq!(attention(p.store())[0].reason, AttentionReason::Safety);
    p.advance(3000);
    assert_eq!(lifecycle(&p, "m-key"), Lifecycle::NeedsMe, "warning must stay until verified");

    p.advance(2000);

    assert_clean(&p);
    assert_eq!(p.store().participant(&mira).expect("mira").safety, SafetyState::Verified);
    assert_eq!(lifecycle(&p, "m-key"), Lifecycle::Done);
    assert!(attention_ids(&p).is_empty());
}

#[test]
fn s05_member_removal_requires_confirmation_then_applies() {
    let mut p = player("s05_group_governance");
    p.advance(3500);
    assert_eq!(attention_ids(&p), ["m-remove"]);
    let pending = p.dispatch(approve("g-remove", false)).expect("approve");
    assert!(matches!(pending.as_slice(), [Effect::RequireConfirmation { .. }]));
    assert!(p.dispatch(approve("g-remove", true)).expect("approve").is_empty());
    p.advance(500);

    assert_clean(&p);
    let members: Vec<_> = participants(p.store(), &ContextId::new("c-garden"))
        .iter()
        .map(|m| m.id.to_string())
        .collect();
    assert_eq!(members, ["p-me", "p-lena", "p-mira"]);
    assert_eq!(lifecycle(&p, "m-remove"), Lifecycle::Done);
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
    let resolve = IntentKind::ResolveConflict {
        moment_id: MomentId::new("m-conflict"),
        resolution: ConflictResolution::KeepBoth,
    };
    assert!(p.dispatch(Intent::human(resolve)).expect("resolve").is_empty());
    let reveal = IntentKind::Reveal { moment_id: MomentId::new("m-iban") };
    assert!(p.dispatch(Intent::human(reveal)).expect("reveal").is_empty());
    p.advance(6000);

    assert_clean(&p);
    let store = p.store();
    let conflict = store.moment(&MomentId::new("m-conflict")).expect("conflict");
    let MomentKind::Conflict(payload) = &conflict.kind else { panic!("not a conflict") };
    assert_eq!(payload.resolution, Some(ConflictResolution::KeepBoth));
    let iban = store.moment(&MomentId::new("m-iban")).expect("reveal");
    let MomentKind::PrivacyReveal(payload) = &iban.kind else { panic!("not a reveal") };
    assert_eq!(payload.revealed.as_ref().map(|r| r.by.as_str()), Some("p-me"));
    assert!(payload.content.is_some());
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
