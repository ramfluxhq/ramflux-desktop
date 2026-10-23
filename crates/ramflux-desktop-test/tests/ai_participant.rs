// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The AI participant scenarios replayed through the store: demo 3 end to
//! end (ask, streamed reply, proposal, approval, result patch, done) and its
//! deny and cancel branches, the stale and conflicting surface changes, the
//! renderer limits, and the plain-text timeline of each.

use ramflux_desktop_core::conformance::LimitKind;
use ramflux_desktop_core::selectors::{AttentionReason, attention, in_flight};
use ramflux_desktop_core::*;
use ramflux_desktop_test::{ScenarioPlayer, load_fixture};

fn player(name: &str) -> ScenarioPlayer {
    let scenario = load_fixture(name).unwrap_or_else(|e| panic!("{name}: {e}"));
    ScenarioPlayer::new(&scenario)
}

fn assert_clean(p: &ScenarioPlayer) {
    assert!(p.errors().is_empty(), "apply errors: {:?}", p.errors());
    assert!(p.store().rejections().is_empty(), "{:?}", p.store().rejections());
}

fn ask(p: &mut ScenarioPlayer) {
    let send = IntentKind::SendMessage {
        context_id: ContextId::new("c-launch"),
        text: "@Orbit can you check the launch checklist and sort out what is open?".into(),
    };
    assert!(p.dispatch(Intent::human(send)).expect("send").is_empty());
}

fn surface(p: &ScenarioPlayer, moment: &str) -> (AgentSurfacePayload, SurfaceRecord) {
    let id = MomentId::new(moment);
    let payload = match &p.store().moment(&id).unwrap_or_else(|| panic!("{moment}")).kind {
        MomentKind::AgentSurface(s) => s.clone(),
        other => panic!("{moment} is {other:?}"),
    };
    (payload, p.store().surface(&id).cloned().expect("record"))
}

fn reply_text(p: &ScenarioPlayer) -> String {
    surface(p, "m-reply").0.blocks[0].data["text"].as_str().unwrap_or_default().to_owned()
}

fn task(p: &ScenarioPlayer) -> Task {
    p.store().task(&TaskId::new("t-launch")).cloned().expect("task")
}

fn task_lifecycle(p: &ScenarioPlayer) -> Lifecycle {
    p.store().moment(&MomentId::new("m-task")).expect("task moment").lifecycle
}

fn held(intent: Intent) -> Intent {
    intent.confirmed_by_human(ConfirmationIssuer::new().issue_hold(1_200).expect("a full hold"))
}

/// Plays demo 3 up to the approval the deploy step needs.
fn to_approval() -> ScenarioPlayer {
    let mut p = player("s10_ai_participant");
    p.advance(200);
    ask(&mut p);
    p.advance(1_100);
    let accept = IntentKind::AcceptTaskProposal { task_id: TaskId::new("t-launch") };
    assert!(p.dispatch(Intent::human(accept)).expect("start").is_empty());
    p.advance(1_400);
    p
}

#[test]
fn s10_streams_the_reply_then_proposes_a_task() {
    let mut p = player("s10_ai_participant");
    p.advance(200);
    ask(&mut p);
    p.advance(300);
    let (reply, record) = surface(&p, "m-reply");
    assert!(reply.streaming, "an empty reply waits for its stream");
    assert_eq!((reply_text(&p).as_str(), record.version), ("", 1));
    p.advance(200);
    let (reply, record) = surface(&p, "m-reply");
    assert!(reply.streaming, "still writing");
    assert_eq!(reply_text(&p), "I checked the launch checklist. ");
    assert_eq!(record.version, 2);
    p.advance(200);
    assert!(reply_text(&p).ends_with("**the staging deploy**. "));
    p.advance(200);
    let (reply, record) = surface(&p, "m-reply");
    assert!(!reply.streaming, "the final append ends the stream");
    assert_eq!(record.version, 4);
    assert!(reply_text(&p).ends_with("run the deploy for you."));
    p.advance(100);
    assert_eq!(task(&p).state, TaskState::Proposed);
    assert_eq!(task_lifecycle(&p), Lifecycle::NeedsMe);
    let items = attention(p.store());
    assert_eq!(items.len(), 1);
    assert_eq!(
        (items[0].moment_id.as_str(), items[0].reason),
        ("m-task", AttentionReason::TaskProposal)
    );
    assert_clean(&p);
}

#[test]
fn s10_demo_runs_from_the_question_to_done() {
    let mut p = to_approval();
    let t = task(&p);
    assert_eq!(t.state, TaskState::NeedsMe);
    assert_eq!(t.steps[0].state, StepState::Done);
    assert_eq!(t.needs_me_step().map(|(ix, _)| ix), Some(1));
    assert_eq!(task_lifecycle(&p), Lifecycle::NeedsMe);
    assert_eq!(attention(p.store())[0].reason, AttentionReason::TaskInput);
    // High risk: the approval waits for the hold, and never runs without it.
    let approve = || Intent::human(IntentKind::Approve { grant_id: GrantId::new("g-deploy") });
    let pending = p.dispatch(approve()).expect("approve");
    assert!(matches!(pending.as_slice(), [Effect::RequireConfirmation { .. }]));
    assert!(p.source().effects().iter().all(|e| e.label != methods::GRANT_APPROVE));
    p.dispatch(held(approve())).expect("approved with a hold");
    p.advance(900);
    let (status, record) = surface(&p, "m-status");
    assert_eq!(record.version, 2, "the result patch applied on the version Orbit saw");
    assert_eq!(record.state, SurfaceState::Open);
    assert!(status.blocks.iter().any(|b| b.id.as_str() == "b-done"));
    p.advance(700);
    let t = task(&p);
    assert_eq!(t.state, TaskState::Done);
    assert_eq!(t.result_ref, Some(SurfaceId::new("s-status")));
    assert_eq!(task_lifecycle(&p), Lifecycle::Done);
    assert!(attention(p.store()).is_empty());
    assert!(in_flight(p.store()).is_empty());
    assert_clean(&p);
}

#[test]
fn s10_hashes_match_the_contract() {
    let mut p = to_approval();
    let (_, record) = surface(&p, "m-status");
    let scenario = load_fixture("s10_ai_participant").expect("fixture");
    let base = scenario
        .responses
        .iter()
        .flat_map(|r| &r.emit)
        .find_map(|(_, e)| match e {
            DomainEvent::Surface(SurfaceEvent::Patch { surface_id, base_hash, .. })
                if surface_id.as_str() == "s-status" =>
            {
                Some(base_hash.clone())
            }
            _ => None,
        })
        .expect("the result patch");
    assert_eq!(base, record.hash, "set base_hash of the s-status patch to {}", record.hash);
    p.dispatch(held(Intent::human(IntentKind::Approve { grant_id: GrantId::new("g-deploy") })))
        .expect("approve");
    p.advance(2_000);
    assert!(!p.store().surface_conflict(&MomentId::new("m-status")));
}

#[test]
fn s10_deny_fails_the_task_and_skips_the_rest() {
    let mut p = to_approval();
    p.dispatch(Intent::human(IntentKind::Deny { grant_id: GrantId::new("g-deploy") }))
        .expect("deny");
    p.advance(500);
    let t = task(&p);
    assert_eq!(t.state, TaskState::Failed);
    assert_eq!(t.steps[1].state, StepState::Failed);
    assert_eq!(t.steps[2].state, StepState::Skipped);
    assert_eq!(task_lifecycle(&p), Lifecycle::Failed);
    assert_eq!(surface(&p, "m-status").1.version, 1, "no result to report");
    assert_clean(&p);
}

#[test]
fn s10_the_requester_declines_the_proposal() {
    let mut p = player("s10_ai_participant");
    p.advance(200);
    ask(&mut p);
    p.advance(1_100);
    let agent_cancel = Intent::agent(
        ParticipantId::new("p-orbit"),
        IntentKind::CancelTask { task_id: TaskId::new("t-launch") },
    );
    assert!(matches!(p.dispatch(agent_cancel), Err(IntentError::NotRequester(_))));
    p.dispatch(Intent::human(IntentKind::CancelTask { task_id: TaskId::new("t-launch") }))
        .expect("decline");
    p.advance(200);
    assert_eq!(task(&p).state, TaskState::Cancelled);
    assert_eq!(task_lifecycle(&p), Lifecycle::Dismissed);
    assert!(task(&p).steps.iter().all(|s| s.state == StepState::Skipped));
    assert_clean(&p);
}

#[test]
fn s10_timeline_degrades_to_text() {
    let mut p = to_approval();
    p.dispatch(held(Intent::human(IntentKind::Approve { grant_id: GrantId::new("g-deploy") })))
        .expect("approve");
    p.advance(2_000);
    let lines = export::timeline_text(p.store(), &ContextId::new("c-launch"));
    let text = lines.join("\n");
    assert!(text.contains("Lena Hart: Friday is close."), "{text}");
    assert!(text.contains("Orbit: Orbit: two checklist items are open"), "{text}");
    assert!(text.contains("Task for Orbit: Finish the launch checklist [done]"), "{text}");
    assert!(text.contains("2. Deploy to staging [done]"), "{text}");
    assert!(text.contains("result: Release notes drafted"), "{text}");
    assert!(text.contains("Orbit's launch checklist"), "{text}");
    assert!(lines.iter().all(|l| !l.trim().is_empty()));
    assert!(!text.contains("b-items"), "never block data, only fallback text");
}

#[test]
fn every_surface_and_task_in_every_scenario_has_a_text_fallback() {
    for name in ramflux_desktop_test::SCENARIO_NAMES {
        let mut p = player(name);
        p.run_to_end();
        for context in p.store().contexts() {
            let moments: Vec<&Moment> = p.store().context_moments(&context.id).collect();
            let lines = export::timeline_text(p.store(), &context.id);
            assert_eq!(lines.len(), moments.len());
            for (moment, line) in moments.iter().zip(&lines) {
                let (author, body) = line.split_once(": ").expect("author: text");
                assert!(
                    !author.is_empty() && !body.trim().is_empty(),
                    "{name} {}: {line}",
                    moment.id
                );
                if let MomentKind::AgentSurface(s) = &moment.kind {
                    assert!(body.starts_with(&s.fallback_text), "{name} {}: {line}", moment.id);
                }
            }
        }
    }
}

#[test]
fn over_limit_values_match_the_protocol_budgets() {
    let limits = conformance::Limits::DESKTOP;
    assert_eq!(
        (limits.max_blocks, limits.max_actions, limits.max_text_chars, limits.max_data_bytes),
        (64, 16, 16_384, 65_536)
    );
    assert_eq!((limits.max_text_delta_bytes, limits.max_stream_chars), (4_096, 16_384));
    let _ = LimitKind::TextDelta;
}

/// Every `base_hash` in the scenario's timeline that patches `surface`.
fn timeline_bases(name: &str, surface: &str) -> Vec<(u64, SurfaceHash)> {
    load_fixture(name)
        .expect("fixture")
        .timeline
        .iter()
        .filter_map(|(at, e)| match e {
            DomainEvent::Surface(SurfaceEvent::Patch { surface_id, base_hash, .. })
                if surface_id.as_str() == surface =>
            {
                Some((*at, base_hash.clone()))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn s11_hashes_match_the_contract() {
    let mut p = player("s11_surface_versions");
    let bases = timeline_bases("s11_surface_versions", "s-pick");
    p.advance(300);
    let v1 = surface(&p, "m-pick").1.hash;
    assert_eq!(bases[0].1, v1, "set the 1500 ms base_hash to {v1}");
    assert_eq!(bases[1].1, v1, "the 2500 ms patch is made against v1 (the conflict): {v1}");
    p.advance(1_300);
    let v2 = surface(&p, "m-pick").1.hash;
    assert_eq!(bases[2].1, v2, "set the 3500 ms base_hash to {v2}");
    p.advance(2_000);
    let v3 = surface(&p, "m-pick").1.hash;
    let scenario = load_fixture("s11_surface_versions").expect("fixture");
    let answers: Vec<&SurfaceHash> = scenario
        .responses
        .iter()
        .flat_map(|r| &r.emit)
        .filter_map(|(_, e)| match e {
            DomainEvent::Surface(SurfaceEvent::Patch { base_hash, .. }) => Some(base_hash),
            _ => None,
        })
        .collect();
    assert!(answers.iter().all(|h| **h == v3), "set the answer patches' base_hash to {v3}");
}

fn pick(p: &ScenarioPlayer, action: &str) -> Intent {
    let (_, record) = surface(p, "m-pick");
    Intent::human(IntentKind::SurfaceAction {
        moment_id: MomentId::new("m-pick"),
        surface_id: SurfaceId::new("s-pick"),
        action_id: action.to_owned(),
        values: serde_json::Value::Null,
        surface_hash: record.hash,
        version: record.version,
    })
}

#[test]
fn s11_a_choice_waits_at_its_surface_survives_a_conflict_and_closes() {
    let mut p = player("s11_surface_versions");
    p.advance(300);
    let items = attention(p.store());
    assert_eq!(items.len(), 1);
    assert_eq!(
        (items[0].moment_id.as_str(), items[0].reason),
        ("m-pick", AttentionReason::SurfaceInput)
    );
    // A high-risk choice waits for the hold; the change at 1500 ms makes the
    // reviewed version stale, so the held intent is refused, never resent.
    let shown = pick(&p, "fri");
    let IntentKind::SurfaceAction { surface_hash: seen, .. } = &shown.kind else { unreachable!() };
    let seen = seen.clone();
    assert!(matches!(
        p.dispatch(shown).expect("fri").as_slice(),
        [Effect::RequireConfirmation { .. }]
    ));
    p.advance(1_300);
    let stale = Intent::human(IntentKind::SurfaceAction {
        moment_id: MomentId::new("m-pick"),
        surface_id: SurfaceId::new("s-pick"),
        action_id: "fri".to_owned(),
        values: serde_json::Value::Null,
        surface_hash: seen,
        version: 1,
    });
    assert!(matches!(p.dispatch(held(stale)), Err(IntentError::SurfaceChanged(_))));
    assert_eq!(surface(&p, "m-pick").1.version, 2);
    // 2500 ms: a change made against v1 leaves the surface out of date.
    p.advance(1_000);
    assert!(p.store().surface_conflict(&MomentId::new("m-pick")));
    assert!(matches!(p.dispatch(pick(&p, "thu")), Err(IntentError::SurfaceConflict(_))));
    // 3500 ms: resent against v2, applied.
    p.advance(1_000);
    let (payload, record) = surface(&p, "m-pick");
    assert_eq!((record.version, &record.state), (3, &SurfaceState::Open));
    assert_eq!(payload.blocks.len(), 2);
    assert!(p.dispatch(pick(&p, "thu")).expect("thursday").is_empty());
    let body = &p.source().effects().last().expect("effect").body;
    assert_eq!((body["version"].as_u64(), body["action_id"].as_str()), (Some(3), Some("thu")));
    assert!(body["args_hash"].as_str().is_some_and(|h| h.starts_with("sha256:")));
    p.advance(1_500);
    let (payload, record) = surface(&p, "m-pick");
    assert_eq!((record.version, &record.state), (5, &SurfaceState::Closed));
    assert!(payload.blocks.iter().any(|b| b.id.as_str() == "b-answer"));
    assert!(matches!(p.dispatch(pick(&p, "mon")), Err(IntentError::SurfaceClosed(_))));
    let t = p.store().task(&TaskId::new("t-day")).expect("task");
    assert_eq!(t.state, TaskState::Done);
    assert_eq!(t.steps[0].needs_me, Some(NeedsMe::Choice { surface_id: SurfaceId::new("s-pick") }));
    assert!(attention(p.store()).is_empty());
    assert_eq!(p.errors(), &[], "every event applied");
    let text = export::timeline_text(p.store(), &ContextId::new("c-plan")).join("\n");
    assert!(text.contains("Orbit asks which day the release goes out. [closed]"), "{text}");
}

#[test]
fn s12_limits_leave_only_the_fallback_text() {
    let mut p = player("s12_surface_limits");
    p.advance(150);
    let (many, record) = surface(&p, "m-many");
    assert_eq!(record.state, SurfaceState::OverLimit(LimitKind::Actions));
    assert!(many.blocks.is_empty() && many.actions.is_empty());
    p.advance(500);
    let (log, record) = surface(&p, "m-log");
    assert_eq!(record.version, 3);
    assert_eq!(log.blocks[0].data["text"], "compiling core\ncompiling ui\n");
    p.advance(200);
    let (log, record) = surface(&p, "m-log");
    assert_eq!(record.state, SurfaceState::OverLimit(LimitKind::TextDelta));
    assert!(log.blocks.is_empty(), "never a partial rendering");
    assert_eq!(log.fallback_text, "Orbit streamed the build log of release 2.4.0.");
    p.advance(300);
    assert_eq!(surface(&p, "m-log").1.state, SurfaceState::Closed);
    assert_clean(&p);
}
