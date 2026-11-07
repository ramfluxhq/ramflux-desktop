// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Typed v1 surfaces (S10c c07): the catalog gate and the requester binding
//! of actions and receipts. Only `basic/1` is drawn; every other catalog,
//! version and every unbound answer leaves the author's fallback text with
//! no blocks and no actions in the payload, and nothing on the wire.

mod common;
mod sdk_fake;

use common::*;
use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt as _;
use ramflux_desktop_core::{
    AccountId, Context, ContextId, ContextKind, DomainEvent, EventEnvelope, Intent, IntentError,
    IntentKind, MomentId, MomentKind, Store, SurfaceEvent, Timestamp,
};
use ramflux_desktop_sdk::Scope;
use ramflux_desktop_sdk::translate::Translator;
use ramflux_desktop_sdk::translate::v1::candidate::{apply, surface_counts};
use ramflux_desktop_sdk::translate::v1::surface::{Inert, Route, route};
use ramflux_desktop_test::v1_fixtures::{
    SurfaceSpec, identity, message_update, service_update, surface_update, text_message,
};
use ramflux_desktop_ui::app::DispatchOutcome;
use ramflux_protocol::message::MessageUpdate;
use ramflux_protocol::message::{
    JsonMember, JsonObject, JsonValue, ServiceAction, SurfaceCarrier, json_value,
    surface_carrier::Payload as CarrierPayload, surface_command::Value as Command,
};

const DM: &str = "dm-ana";
const AUTHOR: &str = "principal-ana";

/// The scoped reference of a surface, as the daemon scopes it.
fn scoped(spec: &SurfaceSpec) -> String {
    ramflux_desktop_sdk::surface::surface_ref(&spec.conversation, &spec.author, &spec.surface_id)
        .expect("canonical ids")
}

fn moment_of(spec: &SurfaceSpec) -> MomentId {
    MomentId::new(format!("surface:{}:{}", spec.conversation, scoped(spec)))
}

fn update(n: u8, carrier: ramflux_protocol::message::SurfaceCarrier) -> MessageUpdate {
    surface_update(identity(DM, &[n; 4], "dev-ana", u64::from(n) * 10, u64::from(n)), carrier)
}

fn update_in(conversation: &str, n: u8, carrier: SurfaceCarrier) -> MessageUpdate {
    surface_update(
        identity(conversation, &[n; 4], "dev-ana", u64::from(n) * 10, u64::from(n)),
        carrier,
    )
}

fn created(n: u8, spec: &SurfaceSpec) -> MessageUpdate {
    update(n, spec.create_carrier(&format!("m-{n}")))
}

fn translator() -> Translator {
    Translator::new(Scope::Account(AccountId::new("acct-main")), "/tmp/s10c-surface.sock")
}

/// The surface snapshots `updates` project to, in order.
fn snapshots(translator: &mut Translator, updates: &[MessageUpdate]) -> Vec<SurfaceEvent> {
    apply(translator, updates)
        .events
        .into_iter()
        .filter_map(|event| match event {
            DomainEvent::Surface(event) => Some(event),
            _ => None,
        })
        .collect()
}

fn snapshot_of(event: &SurfaceEvent) -> &ramflux_desktop_core::SurfaceSnapshot {
    match event {
        SurfaceEvent::Snapshot(snapshot) => snapshot,
        other => panic!("not a snapshot: {other:?}"),
    }
}

const SECRET_KEYS: [&str; 9] = [
    "key_slots",
    "encrypted_key",
    "read_capability",
    "raw_handle",
    "group_key",
    "key_distribution",
    "decision_proof",
    "raw_inner",
    "inner_bytes",
];

fn keys_of(value: &serde_json::Value, found: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, inner) in map {
                found.push(key.clone());
                keys_of(inner, found);
            }
        }
        serde_json::Value::Array(items) => items.iter().for_each(|item| keys_of(item, found)),
        _ => {}
    }
}

fn assert_no_secret_keys(events: &[DomainEvent]) {
    let mut keys = Vec::new();
    keys_of(&serde_json::to_value(events).expect("events serialize"), &mut keys);
    for key in keys {
        assert!(!SECRET_KEYS.contains(&key.as_str()), "secret field `{key}` in a UI model");
    }
}

#[test]
fn routes_only_basic_one_to_the_native_registry() {
    assert_eq!(route("basic", "1"), Route::Native);
    assert_eq!(route("rf-web/v1", "1"), Route::Inert(Inert::WebInactive));
    assert_eq!(route("rf-web/v1", "9"), Route::Inert(Inert::UnknownVersion));
    assert_eq!(route("basic", "2"), Route::Inert(Inert::UnknownVersion));
    assert_eq!(route("vendor", "1"), Route::Inert(Inert::UnknownCatalog));
}

#[test]
fn basic_projects_blocks_and_actions_and_others_project_fallback_only() {
    let basic = SurfaceSpec::basic(DM, "s-basic", AUTHOR);
    let mut t = translator();
    let shown = snapshots(&mut t, &[created(1, &basic)]);
    let native = snapshot_of(&shown[0]);
    assert_eq!(native.blocks.len(), 1);
    assert_eq!(native.actions.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["open"]);
    assert_eq!(native.hash.as_str(), "hash-s-basic-1", "the attested hash");
    for (catalog, version) in [("rf-web/v1", "1"), ("vendor", "1"), ("basic", "2")] {
        let spec = SurfaceSpec::basic(DM, "s-other", AUTHOR).in_catalog(catalog, version);
        let mut t = translator();
        let shown = snapshots(&mut t, &[created(2, &spec)]);
        let inert = snapshot_of(&shown[0]);
        assert!(inert.blocks.is_empty(), "{catalog}/{version}: no blocks");
        assert!(inert.actions.is_empty(), "{catalog}/{version}: actions=[]");
        assert_eq!(inert.fallback_text, "Report is ready");
    }
}

/// A text block whose value nests `levels` arrays: the component object plus
/// `levels` arrays is `1 + levels` containers deep.
fn nested(levels: usize) -> SurfaceSpec {
    let mut deep = serde_json::json!("leaf");
    for _ in 0..levels {
        deep = serde_json::json!([deep]);
    }
    let mut spec = SurfaceSpec::basic(DM, "s-deep", AUTHOR);
    spec.components = vec![serde_json::json!({ "id": "b", "type": "Text", "text": deep })];
    spec
}

fn counted(components: usize, actions: usize) -> SurfaceSpec {
    let mut spec = SurfaceSpec::basic(DM, "s-count", AUTHOR);
    spec.components = (0..components)
        .map(|n| serde_json::json!({ "id": format!("b-{n}"), "type": "Text", "text": "x" }))
        .collect();
    spec.actions = (0..actions).map(|n| serde_json::json!({ "name": format!("a-{n}") })).collect();
    spec
}

/// The first component's object members, to edit the wire form directly.
fn first_members(carrier: &mut SurfaceCarrier) -> &mut Vec<JsonMember> {
    let Some(CarrierPayload::Event(event)) = &mut carrier.payload else { panic!("event") };
    let Some(Command::CreateSurface(create)) =
        event.command.as_mut().and_then(|c| c.value.as_mut())
    else {
        panic!("create")
    };
    let value = create.value.as_mut().expect("create value").components[0]
        .value
        .as_mut()
        .expect("component");
    let json_value::Value::Object(JsonObject { members }) = value else { panic!("object") };
    members
}

fn is_inert(spec: &SurfaceSpec) -> bool {
    let shown = snapshots(&mut translator(), &[created(3, spec)]);
    let snapshot = snapshot_of(&shown[0]);
    snapshot.blocks.is_empty() && snapshot.actions.is_empty()
}

#[test]
fn component_and_action_bounds_are_exact() {
    // Depth: 16 containers are read, 17 are refused.
    assert!(!is_inert(&nested(15)), "depth 16 renders");
    assert!(is_inert(&nested(16)), "depth 17 is inert");
    // Count: 512 components are read, 513 are refused.
    assert!(!is_inert(&counted(512, 1)), "512 components render");
    assert!(is_inert(&counted(513, 1)), "513 components are inert");
    // A repeated action id refuses the whole surface.
    let mut repeated = SurfaceSpec::basic(DM, "s-act", AUTHOR);
    repeated.actions.push(repeated.actions[0].clone());
    assert!(is_inert(&repeated), "duplicate action");
    // A repeated block id too, and a block without an id.
    let mut twice = SurfaceSpec::basic(DM, "s-dup", AUTHOR);
    twice.components.push(twice.components[0].clone());
    assert!(is_inert(&twice), "duplicate block");
    let mut no_id = SurfaceSpec::basic(DM, "s-noid", AUTHOR);
    no_id.components = vec![serde_json::json!({ "type": "Text", "text": "x" })];
    assert!(is_inert(&no_id), "no id");
    // A repeated object member in the wire form.
    let basic = SurfaceSpec::basic(DM, "s-member", AUTHOR);
    let mut carrier = basic.create_carrier("m-1");
    let members = first_members(&mut carrier);
    let copy = members[0].clone();
    members.push(copy);
    let shown = snapshots(&mut translator(), &[update(3, carrier)]);
    let snapshot = snapshot_of(&shown[0]);
    assert!(snapshot.blocks.is_empty() && snapshot.actions.is_empty(), "duplicate member");
    // A member without a value, and a number that is not one.
    let mut carrier = basic.create_carrier("m-1");
    first_members(&mut carrier)[0].value = None;
    let shown = snapshots(&mut translator(), &[update(3, carrier)]);
    assert!(snapshot_of(&shown[0]).blocks.is_empty(), "member without a value");
    let mut carrier = basic.create_carrier("m-1");
    first_members(&mut carrier).push(JsonMember {
        key: "n".to_owned(),
        value: Some(JsonValue {
            value: Some(json_value::Value::Number("not-a-number".to_owned())),
        }),
    });
    let shown = snapshots(&mut translator(), &[update(3, carrier)]);
    assert!(snapshot_of(&shown[0]).blocks.is_empty(), "bad number");
}

#[test]
fn replay_never_rolls_back_and_an_inert_surface_never_revives() {
    let mut t = translator();
    let v2 = SurfaceSpec::basic(DM, "s-replay", AUTHOR).at_version(2);
    let v1 = SurfaceSpec::basic(DM, "s-replay", AUTHOR);
    assert_eq!(snapshots(&mut t, &[created(1, &v2)]).len(), 1);
    assert!(snapshots(&mut t, &[created(2, &v1)]).is_empty(), "lower version ignored");
    assert!(snapshots(&mut t, &[created(3, &v2)]).is_empty(), "same version ignored");
    // An inert surface stays inert at any later version.
    let web = SurfaceSpec::basic(DM, "s-web", AUTHOR).in_catalog("rf-web/v1", "1");
    assert_eq!(snapshots(&mut t, &[created(4, &web)]).len(), 1);
    let native = SurfaceSpec::basic(DM, "s-web", AUTHOR).at_version(3);
    assert!(snapshots(&mut t, &[created(5, &native)]).is_empty(), "no revival");
}

#[test]
fn receipts_bind_to_the_original_requester_only() {
    let spec = SurfaceSpec::basic(DM, "s-bound", AUTHOR);
    let action = |requester: &str| spec.action_carrier("act-1", requester, "open", 1);
    // The matching requester: accepted, the surface keeps its actions.
    let mut t = translator();
    let ok = snapshots(
        &mut t,
        &[
            created(1, &spec),
            update(2, action("principal-bo")),
            update(3, spec.receipt_carrier("rcpt-1", action("principal-bo"))),
        ],
    );
    assert_eq!(ok.len(), 1, "only the creation");
    assert_eq!(surface_counts(&t), (1, 0), "the pending action was consumed");
    // Another requester: the surface is neutralised.
    let mut t = translator();
    let bad = snapshots(
        &mut t,
        &[
            created(1, &spec),
            update(2, action("principal-bo")),
            update(3, spec.receipt_carrier("rcpt-1", action("principal-cy"))),
        ],
    );
    let last = snapshot_of(bad.last().expect("neutralised"));
    assert!(last.actions.is_empty() && last.blocks.is_empty());
    assert_eq!(last.version, 1);
    // A receipt for an action never recorded changes nothing.
    let mut t = translator();
    let unknown = snapshots(
        &mut t,
        &[created(1, &spec), update(3, spec.receipt_carrier("rcpt-1", action("principal-cy")))],
    );
    assert_eq!(unknown.len(), 1);
    // The same action named by a second requester.
    let mut t = translator();
    let twice = snapshots(
        &mut t,
        &[created(1, &spec), update(2, action("principal-bo")), update(3, action("principal-cy"))],
    );
    assert!(snapshot_of(twice.last().expect("neutralised")).actions.is_empty());
    // Each binding operand, changed alone from a valid receipt.
    type Change = Box<dyn Fn(&mut SurfaceCarrier)>;
    let operands: [(&str, Change); 5] = [
        ("inner surface id", Box::new(|c| c.surface_id = "s-other".to_owned())),
        ("inner author", Box::new(|c| c.author_principal_id = "principal-cy".to_owned())),
        ("inner conversation", Box::new(|c| c.conversation_id = "c-elsewhere".to_owned())),
        (
            "inner action surface id",
            Box::new(|c| {
                if let Some(CarrierPayload::Action(action)) = &mut c.payload {
                    action.surface_id = "s-other".to_owned();
                }
            }),
        ),
        (
            "inner payload kind",
            Box::new(|c| {
                c.payload = Some(CarrierPayload::ActionReceipt(Box::new(c.clone())));
            }),
        ),
    ];
    for (operand, change) in operands {
        let mut inner = action("principal-bo");
        change(&mut inner);
        let mut t = translator();
        let out = snapshots(
            &mut t,
            &[
                created(1, &spec),
                update(2, action("principal-bo")),
                update(3, spec.receipt_carrier("rcpt-1", inner)),
            ],
        );
        let last = snapshot_of(out.last().expect("neutralised"));
        assert_eq!(out.len(), 2, "{operand}: exactly one neutralising snapshot");
        assert!(last.actions.is_empty() && last.blocks.is_empty(), "{operand}");
    }
    // The receipt's own author names another surface: it is not this one.
    let mut foreign = spec.receipt_carrier("rcpt-3", action("principal-bo"));
    foreign.author_principal_id = "principal-cy".to_owned();
    let mut t = translator();
    let unbound = snapshots(
        &mut t,
        &[created(1, &spec), update(2, action("principal-bo")), update(3, foreign)],
    );
    assert_eq!(unbound.len(), 1, "another author's surface is a different surface");
}

#[test]
fn an_envelope_that_disagrees_with_its_carrier_projects_nothing() {
    let spec = SurfaceSpec::basic("c-elsewhere", "s-env", AUTHOR);
    let shown = snapshots(&mut translator(), &[created(1, &spec)]);
    assert!(shown.is_empty(), "the carrier names another conversation");
}

#[test]
fn retained_surfaces_and_pending_actions_are_bounded_and_cleared_on_lock() {
    let mut t = translator();
    for n in 0..520_u32 {
        let spec = SurfaceSpec::basic(DM, &format!("s-{n}"), AUTHOR);
        let _ = snapshots(
            &mut t,
            &[surface_update(
                identity(DM, &n.to_be_bytes(), "dev-ana", u64::from(n), u64::from(n)),
                spec.create_carrier("m"),
            )],
        );
    }
    assert_eq!(surface_counts(&t).0, 512);
    let spec = SurfaceSpec::basic(DM, "s-0", AUTHOR);
    for n in 0..300_u32 {
        let carrier = spec.action_carrier(&format!("a-{n}"), "principal-bo", "open", 1);
        let _ = snapshots(&mut t, &[update(9, carrier)]);
    }
    assert_eq!(surface_counts(&t).1, 256, "oldest pending actions are dropped");
    let _ = t.own_account_locked();
    assert_eq!(surface_counts(&t), (0, 0), "cleared on lock");
}

#[test]
fn emitted_events_carry_no_secret_keys() {
    let spec = SurfaceSpec::basic(DM, "s-keys", AUTHOR);
    let events = apply(&mut translator(), &[created(1, &spec)]).events;
    assert!(!events.is_empty());
    assert_no_secret_keys(&events);
}

fn open_dm(
    cx: &mut TestAppContext,
    name: &str,
) -> (
    ramflux_desktop_test::fake_bus::FakeBus,
    std::path::PathBuf,
    gpui_kit::WindowHandle<gpui_kit::base::Root>,
    gpui_kit::Entity<ramflux_desktop_ui::Shell>,
) {
    let (fake, _state, path) = sdk_fake::start(name);
    let (handle, shell) = open_with(cx, sdk_fake::options(&path));
    cx.update(|cx| ramflux_desktop_ui::i18n::select_locale("en", cx));
    wait(cx, "workspace", |cx| page(cx, &shell) == ramflux_desktop_ui::route::Page::Workspace);
    wait(cx, "the conversation", |cx| {
        with_store(cx, &shell, |s| s.context(&ContextId::new(DM)).is_some())
    });
    show_context(cx, handle, &shell, DM);
    (fake, path, handle, shell)
}

/// The shown surface's actions and blocks, once its moment is in the store.
fn shown(
    cx: &mut TestAppContext,
    shell: &gpui_kit::Entity<ramflux_desktop_ui::Shell>,
    spec: &SurfaceSpec,
) -> (Vec<String>, usize) {
    let id = moment_of(spec);
    wait(cx, "the surface moment", |cx| with_store(cx, shell, |s| s.moment(&id).is_some()));
    with_store(cx, shell, |s| match s.moment(&id).map(|m| m.kind.clone()) {
        Some(MomentKind::AgentSurface(p)) => {
            (p.actions.iter().map(|a| a.id.clone()).collect(), p.blocks.len())
        }
        other => panic!("not a surface: {other:?}"),
    })
}

/// A forged click: the intent for `open` on the surface as shown.
fn forge(
    cx: &mut TestAppContext,
    shell: &gpui_kit::Entity<ramflux_desktop_ui::Shell>,
    spec: &SurfaceSpec,
) -> DispatchOutcome {
    let id = moment_of(spec);
    let (surface_id, hash, version) = with_store(cx, shell, |s| {
        let record = s.surface(&id).expect("surface record");
        (record.surface_id.clone(), record.hash.clone(), record.version)
    });
    let store = store_handle(cx, shell);
    cx.update(|cx| {
        store.update(cx, |store, cx| {
            store.dispatch(
                Intent::human(IntentKind::SurfaceAction {
                    moment_id: id,
                    surface_id,
                    action_id: "open".to_owned(),
                    values: serde_json::Value::Null,
                    surface_hash: hash,
                    version,
                }),
                cx,
            )
        })
    })
}

/// Scrolls the row of `spec` into view.
fn reveal(
    cx: &mut TestAppContext,
    shell: &gpui_kit::Entity<ramflux_desktop_ui::Shell>,
    spec: &SurfaceSpec,
) {
    let id = moment_of(spec);
    let conversation = workspace(cx, shell).read_with(cx, |w, _| w.conversation().clone());
    cx.update(|cx| conversation.update(cx, |c, cx| c.focus_moment(&id, cx)));
    advance(cx, 50);
}

fn wire_actions(fake: &ramflux_desktop_test::fake_bus::FakeBus) -> usize {
    fake.seen().iter().filter(|seen| seen.method == "surface.action").count()
}

#[gpui_kit::test]
fn v1_surface_projection_catalog_gate_and_answer_binding(cx: &mut TestAppContext) {
    let _locale = locale_guard();
    let (fake, path, handle, shell) = open_dm(cx, "s10c-surface");

    // basic/1 renders natively: its block and its action button.
    let basic = SurfaceSpec::basic(DM, "s-basic", AUTHOR);
    assert!(sdk_fake::push_generated(&path, created(1, &basic)));
    assert_eq!(shown(cx, &shell, &basic), (vec!["open".to_owned()], 1));
    let scope = format!("moment-{}", moment_of(&basic));
    wait(cx, "the native card", |cx| exists_in(cx, handle, &scope, "block-b-text"));
    assert!(exists_in(cx, handle, &scope, "action-open"), "the action is offered");

    // Each negative starts from the same valid surface and changes one operand.
    let negatives = [
        ("web inactive", SurfaceSpec::basic(DM, "s-web", AUTHOR).in_catalog("rf-web/v1", "1")),
        ("unknown catalog", SurfaceSpec::basic(DM, "s-vendor", AUTHOR).in_catalog("vendor", "1")),
        ("unknown version", SurfaceSpec::basic(DM, "s-v2", AUTHOR).in_catalog("basic", "2")),
    ];
    for (n, (row, spec)) in negatives.iter().enumerate() {
        let n = u8::try_from(n).expect("small") + 10;
        assert!(sdk_fake::push_generated(&path, created(n, spec)), "{row}");
        assert_eq!(shown(cx, &shell, spec), (Vec::new(), 0), "{row}: actions=[] in the payload");
        let scope = format!("moment-{}", moment_of(spec));
        let mut offered = true;
        wait(cx, &format!("the fallback block of {row}"), |cx| {
            reveal(cx, &shell, spec);
            let drawn =
                exists(cx, handle, &scope) && exists_in(cx, handle, &scope, "block-fallback");
            if drawn {
                offered = exists_in(cx, handle, &scope, "action-open");
            }
            drawn
        });
        assert!(!offered, "{row}: no action to click");
        let rendered = in_window(cx, handle, {
            let scope = scope.clone();
            move |window, _| {
                window
                    .within(scope)
                    .try_find("block-fallback-text")
                    .and_then(|e| e.label().map(str::to_owned))
            }
        });
        assert_eq!(rendered.as_deref(), Some("Report is ready"), "{row}: the rendered text");
        assert!(!exists_in(cx, handle, &scope, "action-open"), "{row}: no action to click");
        assert!(
            matches!(
                forge(cx, &shell, spec),
                DispatchOutcome::Refused(IntentError::UnknownAction { .. })
            ),
            "{row}: a forged click is refused"
        );
    }

    // An action from a requester other than the original one.
    let bound = SurfaceSpec::basic(DM, "s-bound", AUTHOR);
    assert!(sdk_fake::push_generated(&path, created(20, &bound)));
    assert_eq!(shown(cx, &shell, &bound).0, ["open"]);
    let asked = bound.action_carrier("act-1", "principal-bo", "open", 1);
    assert!(sdk_fake::push_generated(&path, update(21, asked)));
    let wrong = bound.action_carrier("act-1", "principal-cy", "open", 1);
    assert!(sdk_fake::push_generated(&path, update(22, bound.receipt_carrier("rcpt-1", wrong))));
    let id = moment_of(&bound);
    wait(cx, "the surface neutralised", |cx| {
        with_store(cx, &shell, |s| match s.moment(&id).map(|m| m.kind.clone()) {
            Some(MomentKind::AgentSurface(p)) => p.actions.is_empty() && p.blocks.is_empty(),
            _ => false,
        })
    });
    let scope = format!("moment-{id}");
    let mut offered = true;
    wait(cx, "the fallback block", |cx| {
        reveal(cx, &shell, &bound);
        let drawn = exists(cx, handle, &scope) && exists_in(cx, handle, &scope, "block-fallback");
        if drawn {
            offered = exists_in(cx, handle, &scope, "action-open");
        }
        drawn
    });
    assert!(!offered, "no action after a bad receipt");
    assert!(matches!(
        forge(cx, &shell, &bound),
        DispatchOutcome::Refused(IntentError::UnknownAction { .. })
    ));

    assert_eq!(wire_actions(&fake), 0, "zero surface action requests on the wire");
    fake.shutdown();
}

fn scoped_id(spec: &SurfaceSpec) -> ramflux_desktop_core::SurfaceId {
    ramflux_desktop_core::SurfaceId::new(scoped(spec))
}

fn actions_of(event: &SurfaceEvent) -> Vec<String> {
    snapshot_of(event).actions.iter().map(|a| a.id.clone()).collect()
}

#[test]
fn one_author_and_id_in_two_conversations_stay_apart() {
    let a = SurfaceSpec::basic(DM, "s-same", AUTHOR);
    let b = SurfaceSpec::basic("c-other", "s-same", AUTHOR);
    assert_ne!(scoped(&a), scoped(&b), "the scoped reference names the conversation");
    let create_b = update_in("c-other", 2, b.create_carrier("m-2"));
    let mut t = translator();
    let both = snapshots(&mut t, &[created(1, &a), create_b]);
    assert_eq!(both.len(), 2, "B's surface is not dropped after A's");
    assert_eq!(snapshot_of(&both[0]).surface_id, scoped_id(&a));
    assert_eq!(snapshot_of(&both[1]).surface_id, scoped_id(&b));
    assert_eq!(snapshot_of(&both[1]).context_id, ContextId::new("c-other"));
    assert_eq!(surface_counts(&t).0, 2);
    // Same action message id in both conversations: A's binding is its own.
    let asked_a = a.action_carrier("act-1", "principal-bo", "open", 1);
    let asked_b = b.action_carrier("act-1", "principal-cy", "open", 1);
    assert!(snapshots(&mut t, &[update(3, asked_a), update_in("c-other", 4, asked_b)]).is_empty());
    assert_eq!(surface_counts(&t).1, 2, "two pending actions, one per conversation");
    // B's receipt from the wrong requester neutralises B only.
    let wrong = b.action_carrier("act-1", "principal-bo", "open", 1);
    let out = snapshots(&mut t, &[update_in("c-other", 5, b.receipt_carrier("rcpt-b", wrong))]);
    assert_eq!(out.len(), 1);
    assert_eq!(snapshot_of(&out[0]).surface_id, scoped_id(&b));
    assert!(actions_of(&out[0]).is_empty());
    // A's own receipt from its requester still binds, and A keeps its actions.
    let right = a.action_carrier("act-1", "principal-bo", "open", 1);
    assert!(snapshots(&mut t, &[update(6, a.receipt_carrier("rcpt-a", right))]).is_empty());
    assert_eq!(surface_counts(&t).1, 0);
}

fn context() -> Context {
    Context {
        id: ContextId::new(DM),
        kind: ContextKind::Group,
        title: "Team".to_owned(),
        participants: Vec::new(),
        pinned: false,
        unread: 0,
        muted: false,
        disappearing_secs: None,
        roles: std::collections::BTreeMap::new(),
        epoch: None,
    }
}

#[test]
fn a_surface_sorts_in_the_one_stream_with_messages_and_services() {
    let spec = SurfaceSpec::basic(DM, "s-order", AUTHOR);
    let message = |n: u8, hlc: u64| {
        message_update(identity(DM, &[n; 4], "dev-ana", hlc, hlc), text_message("m", Vec::new()))
    };
    let pin = service_update(
        identity(DM, &[4; 4], "dev-ana", 120, 120),
        ServiceAction::ServicePinChanged,
        &[],
        None,
    );
    let surface =
        surface_update(identity(DM, &[5; 4], "dev-ana", 150, 150), spec.create_carrier("m-5"));
    // A bad receipt neutralises the card later: it keeps its place.
    let asked = update(6, spec.action_carrier("act-1", "principal-bo", "open", 1));
    let wrong = spec.action_carrier("act-1", "principal-cy", "open", 1);
    let receipt = update(7, spec.receipt_carrier("rcpt-1", wrong));
    // Arrival order is not stream order.
    let events =
        apply(&mut translator(), &[message(3, 200), surface, message(1, 100), pin, asked, receipt])
            .events;
    let mut store = Store::new();
    store
        .apply(&EventEnvelope::new(1, Timestamp(1), DomainEvent::ContextUpserted(context())))
        .expect("context");
    for (seq, event) in events.into_iter().enumerate() {
        store.apply(&EventEnvelope::new(2 + seq as u64, Timestamp(1), event)).expect("event");
    }
    let kinds: Vec<&str> = store
        .context_moments(&ContextId::new(DM))
        .map(|m| match &m.kind {
            MomentKind::Message(_) => "message",
            MomentKind::System(_) => "pin",
            MomentKind::AgentSurface(_) => "surface",
            _ => "other",
        })
        .collect();
    assert_eq!(kinds, ["message", "pin", "surface", "message"], "hlc 100, 120, 150, 200");
    let shown = store.moment(&moment_of(&spec)).expect("surface moment");
    assert_eq!(shown.order.as_ref().map(|o| o.hlc), Some(150));
    let MomentKind::AgentSurface(card) = &shown.kind else { panic!("surface") };
    assert!(card.actions.is_empty(), "neutralised, same place");
}
