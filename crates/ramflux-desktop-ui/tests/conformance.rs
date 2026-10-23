// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The renderer conformance suite: the L0, L1 and L2 fixtures played
//! through an account session (store, policy, source) with golden
//! assertions on the resulting store state, the intents and effects they
//! produce, and the renderer errors found against this client's profile.

use ramflux_desktop_core::conformance::{BlockIssue, LimitKind, RendererErrorKind};
use ramflux_desktop_core::{
    BlockId, BlockKind, ConfirmationIssuer, ContextId, GrantFailure, GrantId, GrantState, Intent,
    IntentError, IntentKind, JsonValue, MIN_HOLD_MS, MomentId, MomentKind, Scope, SurfaceHash,
    UiEvent, UiEventKind, methods,
};
use ramflux_desktop_test::load_conformance;
use ramflux_desktop_ui::app::{AccountSession, DispatchOutcome, InspectorEntry};
use ramflux_desktop_ui::moments::MomentRow;
use ramflux_desktop_ui::moments::surface::registry;
use ramflux_desktop_ui::root::intent_for;
use ramflux_desktop_ui::settings::diagnostics;

/// A session on a conformance fixture, with every inspector record kept.
struct Run {
    session: AccountSession,
    records: Vec<InspectorEntry>,
}

impl Run {
    fn new(name: &str, ms: u64) -> Self {
        let mut run = Self {
            session: AccountSession::new(&load_conformance(name).expect("fixture")),
            records: Vec::new(),
        };
        run.advance(ms);
        run
    }

    fn advance(&mut self, ms: u64) {
        let applied = self.session.advance(ms);
        self.records.extend(applied.records);
    }

    fn dispatch(&mut self, intent: Intent) -> DispatchOutcome {
        let (outcome, records) = self.session.dispatch(intent);
        self.records.extend(records);
        outcome
    }

    fn hash(&self, moment: &str) -> SurfaceHash {
        self.session.store().surface_hash(&MomentId::new(moment)).cloned().expect("surface")
    }

    fn row(&self, moment: &str) -> MomentRow {
        let store = self.session.store();
        MomentRow::build(store, store.moment(&MomentId::new(moment)).expect("moment"), None)
    }

    /// The intent a click on `action` of the rendered `moment` maps to.
    fn click(&self, moment: &str, action: &str) -> Intent {
        let row = self.row(moment);
        let hash = row.surface.expect("surface status").verdict.hash;
        let event = UiEvent::new(
            Scope::Block(BlockId::new("b-buttons")),
            UiEventKind::ActionInvoked {
                action_id: action.to_owned(),
                values: JsonValue::Null,
                surface_hash: hash,
            },
        )
        .within(Scope::Moment(MomentId::new(moment)))
        .within(Scope::Context(ContextId::new("c-conf")))
        .within(Scope::Workspace);
        Intent::human(intent_for(&event, self.session.store()).expect("an intent"))
    }

    fn effects(&self) -> Vec<(String, JsonValue)> {
        self.session.effects().map(|(l, b)| (l.to_owned(), b.clone())).collect()
    }

    fn errors(&self) -> Vec<(String, Option<String>, RendererErrorKind)> {
        self.session
            .renderer_errors()
            .recent()
            .iter()
            .map(|e| {
                (e.moment_id.to_string(), e.block_id.as_ref().map(ToString::to_string), e.kind)
            })
            .collect()
    }
}

fn hold() -> ramflux_desktop_core::HumanConfirmation {
    ConfirmationIssuer::new().issue_hold(MIN_HOLD_MS + 50).expect("a full hold")
}

#[test]
fn l0_text_metric_action_and_fallback() {
    let mut run = Run::new("l0_basics", 200);
    let row = run.row("m-l0");
    let verdict = row.surface.clone().expect("surface").verdict;
    assert!(verdict.renders());
    assert_eq!(
        verdict.blocks.into_iter().collect::<Vec<_>>(),
        [(BlockId::new("b-holo"), BlockIssue::Unsupported)],
        "only the unknown kind falls back; the explicit fallback block is supported"
    );
    assert_eq!(
        run.errors(),
        [("m-l0".to_owned(), Some("b-holo".to_owned()), RendererErrorKind::UnsupportedBlock)]
    );

    // The click binds the version it was shown on and reaches the source.
    let shown = run.hash("m-l0");
    let intent = run.click("m-l0", "ack");
    assert!(
        matches!(&intent.kind, IntentKind::SurfaceAction { surface_hash, .. } if *surface_hash == shown)
    );
    assert!(
        matches!(run.dispatch(intent), DispatchOutcome::Submitted(ref l) if l == &[methods::SURFACE_ACTION])
    );
    run.advance(200);
    let MomentKind::AgentSurface(surface) =
        &run.session.store().moment(&MomentId::new("m-l0")).expect("m").kind
    else {
        panic!("not a surface")
    };
    assert_eq!(surface.actions[0].disabled_reason.as_deref(), Some("Acknowledged"));
    assert_ne!(run.hash("m-l0"), shown, "the answer patched a new version");

    // Golden effects: the renderer error report, then the action.
    let effects = run.effects();
    let labels: Vec<&str> = effects.iter().map(|(l, _)| l.as_str()).collect();
    assert_eq!(labels, [methods::RENDERER_ERROR, methods::SURFACE_ACTION]);
    assert_eq!(effects[0].1["error"], "unsupported_block");
    assert_eq!(effects[0].1["block_kind"], "Unsupported");
    assert_eq!(effects[1].1["surface_hash"], serde_json::json!(shown));
    assert_eq!(effects[1].1["action_id"], "ack");
}

#[test]
fn l1_markdown_details_table_fallback_patch_and_snapshot() {
    let mut run = Run::new("l1_documents", 200);
    let first = run.hash("m-l1");
    let verdict = run.row("m-l1").surface.expect("surface").verdict;
    assert_eq!(
        verdict.blocks.into_iter().collect::<Vec<_>>(),
        [
            (BlockId::new("b-broken"), BlockIssue::Schema),
            (BlockId::new("b-key"), BlockIssue::Redacted),
            (BlockId::new("b-v2"), BlockIssue::Unsupported),
        ]
    );
    let kinds: Vec<RendererErrorKind> = run.errors().into_iter().map(|(_, _, k)| k).collect();
    assert_eq!(
        kinds,
        [
            RendererErrorKind::SchemaMismatch,
            RendererErrorKind::UnsupportedBlock,
            RendererErrorKind::PrivacyBlocked,
        ]
    );

    // 500 ms: a streaming patch applies and advances the version.
    run.advance(350);
    let second = run.hash("m-l1");
    assert_ne!(first, second);
    assert!(!run.session.store().surface_conflict(&MomentId::new("m-l1")));

    // 600 ms: a patch against another base is not applied; out of date.
    run.advance(100);
    assert_eq!(run.hash("m-l1"), second, "a stale patch never applies");
    assert!(run.row("m-l1").surface.expect("surface").conflict);
    assert!(
        run.errors()
            .iter()
            .any(|(_, block, k)| block.is_none() && *k == RendererErrorKind::StaleBase)
    );

    // 900 ms: the snapshot replaces the surface and resolves the conflict.
    run.advance(300);
    let status = run.row("m-l1").surface.expect("surface");
    assert!(!status.conflict);
    assert!(status.verdict.blocks.is_empty(), "the snapshot is fully renderable");
    assert_ne!(status.verdict.hash, second);

    // 1000 ms: a surface over the action limit shows only its fallback,
    // and its actions are refused.
    run.advance(100);
    let over = run.row("m-over").surface.expect("surface");
    assert_eq!(over.verdict.over_limit, Some(LimitKind::Actions));
    assert!(over.verdict.blocks.is_empty(), "never a partial rendering");
    let refused = run.dispatch(run.click("m-over", "opt-01"));
    assert!(
        matches!(refused, DispatchOutcome::Refused(IntentError::SurfaceOverLimit(ref m)) if m.as_str() == "m-over")
    );
    assert!(run.errors().contains(&(
        "m-over".to_owned(),
        None,
        RendererErrorKind::OverLimit(LimitKind::Actions)
    )));
    assert!(run.session.store().rejections().is_empty(), "{:?}", run.session.store().rejections());
}

#[test]
fn l2_form_tool_call_approval_and_capability_negotiation() {
    let mut run = Run::new("l2_interactive", 500);
    // Capability negotiation: the source got exactly the registry's profile,
    // and a block version outside it falls back.
    assert_eq!(run.session.negotiated_profile(), Some(registry::profile()));
    let verdict = run.row("m-form").surface.expect("surface").verdict;
    assert_eq!(
        verdict.blocks.into_iter().collect::<Vec<_>>(),
        [(BlockId::new("b-future"), BlockIssue::Unsupported)]
    );

    // Form: a low-risk submit goes straight through with the form values.
    let mut submit = run.click("m-form", "submit");
    if let IntentKind::SurfaceAction { values, .. } = &mut submit.kind {
        *values = serde_json::json!({ "region": "us" });
    }
    assert!(matches!(run.dispatch(submit), DispatchOutcome::Submitted(_)));
    let (_, body) = run.effects().pop().expect("effect");
    assert_eq!(body["values"]["region"], "us");
    run.advance(200);

    // Deploy is high risk: the hold is required, and a patch arriving
    // during the hold invalidates it (policy refuses the confirmed intent).
    let DispatchOutcome::NeedsConfirmation(pending) = run.dispatch(run.click("m-form", "deploy"))
    else {
        panic!("deploy waits for the hold")
    };
    run.advance(2_500);
    let refused = run.dispatch(pending.confirmed_by_human(hold()));
    assert!(
        matches!(refused, DispatchOutcome::Refused(IntentError::SurfaceChanged(ref m)) if m.as_str() == "m-form")
    );
    // Reviewed again on the new version, the hold goes through.
    let DispatchOutcome::NeedsConfirmation(again) = run.dispatch(run.click("m-form", "deploy"))
    else {
        panic!("deploy waits for the hold")
    };
    assert!(matches!(
        run.dispatch(again.confirmed_by_human(hold())),
        DispatchOutcome::Submitted(_)
    ));
    let (label, body) = run.effects().pop().expect("effect");
    assert_eq!(label, methods::SURFACE_ACTION);
    assert_eq!(body["surface_hash"], serde_json::json!(run.hash("m-form")));
    assert_eq!(body["confirmed_by"]["gesture"], "Hold");

    // Approval block: a medium grant decided in place.
    let approve =
        |grant: &str| Intent::human(IntentKind::Approve { grant_id: GrantId::new(grant) });
    assert!(matches!(run.dispatch(approve("g-notice")), DispatchOutcome::Submitted(_)));

    // Tool call: the high-risk approval is refused by the backend and fails
    // closed; nothing retries it and policy refuses it from now on.
    let DispatchOutcome::NeedsConfirmation(tool) = run.dispatch(approve("g-tool")) else {
        panic!("a high-risk approval waits for the hold")
    };
    assert!(matches!(run.dispatch(tool.confirmed_by_human(hold())), DispatchOutcome::Submitted(_)));
    run.advance(300);
    let store = run.session.store();
    assert_eq!(store.grant(&GrantId::new("g-notice")).expect("g").state, GrantState::Approved);
    let tool = store.grant(&GrantId::new("g-tool")).expect("g");
    assert_eq!(tool.failure, Some(GrantFailure::SignedApprovalRequired));
    assert_eq!(tool.state, GrantState::Pending);
    let approvals = run.effects().iter().filter(|(l, _)| l == methods::GRANT_APPROVE).count();
    assert_eq!(approvals, 2, "one approval per grant; the failed one is not retried");
    assert!(matches!(
        run.dispatch(approve("g-tool")),
        DispatchOutcome::Refused(IntentError::GrantFailed { .. })
    ));
    assert!(run.session.store().rejections().is_empty());
}

#[test]
fn redacted_text_never_reaches_debug_inspector_effects_or_diagnostics() {
    const SECRET: &str = "amber-otter-42";
    let mut run = Run::new("l1_documents", 450);
    let store = run.session.store();
    let MomentKind::AgentSurface(surface) = &store.moment(&MomentId::new("m-l1")).expect("m").kind
    else {
        panic!("not a surface")
    };
    let key = surface.blocks.iter().find(|b| b.id.as_str() == "b-key").expect("block");
    assert!(key.redacted && key.kind == BlockKind::Text);
    let outputs = [
        format!("{store:?}"),
        format!("{:?}", run.row("m-l1")),
        format!("{:?}", run.session.renderer_errors()),
        format!("{:?}", run.records),
        format!("{:?}", run.effects()),
        diagnostics::preview(store),
        diagnostics::renderer_summary(registry::profile(), Some(run.session.renderer_errors())),
    ];
    for output in &outputs {
        assert!(!output.contains(SECRET), "redacted text leaked: {output}");
    }
    assert!(store.bodies().is_empty(), "surface blocks never enter the body cache");
    run.advance(1_000);
    assert!(!format!("{:?}", run.session.store()).contains(SECRET));
}
