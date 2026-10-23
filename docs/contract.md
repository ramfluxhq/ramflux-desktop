# Ramflux Desktop data contract (F0)

This document describes the data contract implemented in
`crates/ramflux-desktop-core`. The contract comes before the SDK: scripted
mock data in `fixtures/scenarios` follows it, and SDK integration is measured
against it. If the SDK lacks a capability the contract needs, the SDK gets
extended. The UI does not fall back to the SDK's current shape.

## Layers and rules

```text
ramflux-desktop-ui    views: render selector output, emit UiEvent, hold view-local state only
ramflux-desktop-app   GPUI glue: owns the Store, maps UiEvent -> Intent, hands Effects to a Source
ramflux-desktop-core  pure Rust: model, events, Store, selectors, intent policy, Source trait
Source                MockSource (scenarios) or an SDK-backed source
```

Data flows in one direction:

```text
UiEvent -> Intent -> Effect -> Source -> EventEnvelope -> Store -> selectors -> view
```

| Rule | Meaning |
|---|---|
| R1 | `Store::apply` is the only writer. It applies `EventEnvelope`s and nothing else. |
| R2 | The flow above is one-way. Intent handling never mutates the store. |
| R3 | Core has no GPUI, async runtime, or SDK dependency (only `serde`, `serde_json`, `thiserror`). |
| R4 | View-local state (expanded, hover, scroll) never enters the store. |
| R5 | Optimistic UI uses only a `Pending` moment, which a later event confirms or rolls back. |
| R6 | Humans and agents share one pipeline. Every `Intent` carries an `Actor`. |

## Identifiers and time

`AccountId`, `NodeId`, `ParticipantId`, `ContextId`, `MomentId`, `ObjectId`,
`GrantId`, `TaskId`, `SurfaceId` and `BlockId` are string newtypes that
serialize as plain strings. `Timestamp(i64)` holds unix milliseconds and also
serializes as a plain number. Core never reads the clock. Every time value
comes from a source.

## Model

The store keeps each entity in its own collection, keyed by id:

| Entity | Key fields |
|---|---|
| `Account` | label, node, device, `lock_state` |
| `Node` | endpoints, `pin_source`, `health` |
| `Participant` | `kind` (Human, Agent, Device, Node), `display`, `safety`, `contact` |
| `Context` | `kind` (Direct, Group, Space), `participants`, `pinned`, `unread` |
| `Moment` | `context_id`, `origin`, `at`, `kind: MomentKind`, `lifecycle` |
| `Object` | name, size, direction, `transfer` |
| `Grant` | scope, `risk`, `single_use`, `args_hash`, expiry, `state` |
| `Task` | intent id, context, `actor`, `steps`, `lifecycle` |

`MomentKind` has one variant per inline moment, and each variant carries its
own payload struct: `Message`, `AgentSurface`, `ObjectTransfer`,
`Intervention`, `SafetyChange`, `Governance`, `ContactRequest`,
`TaskProgress`, `Conflict`, `PrivacyReveal`, `Handoff`, `Fallback` and
`System`.

An `AgentSurface` holds `surface_id`, `fallback_text`, an ordered list of
blocks, a list of actions and a `streaming` flag. A `Block` has an `id`, a
`kind: BlockKind` and free-form `data`. `BlockKind` covers the renderer
catalog: Text, Markdown, Heading, KeyValue, Metric, List, TableFallback, Form,
Input, Select, Progress, LogStream, Warning, Error, Success, Approval,
ActionGroup, FilePreview and Fallback. **An unknown kind deserializes to
`Fallback` instead of failing**, and the block's `data` is kept.

An `Intervention` binds a `GrantId` and carries a `risk` (Low, Medium, High
or Critical) and a summary.

### Lifecycle

```text
Pending    -> InProgress | NeedsMe | Done | Failed
InProgress -> NeedsMe | Done | Failed
NeedsMe    -> InProgress | Done | Failed | Dismissed
Done, Failed, Dismissed are terminal
```

`Lifecycle::can_transition(from, to)` encodes this table. Moments, tasks and
task steps all follow it.

## Events

```rust
pub const EVENT_SCHEMA: &str = "ramflux.desktop.event.v1";
pub struct EventEnvelope { pub schema: String, pub seq: u64, pub at: Timestamp, pub event: DomainEvent }
```

`DomainEvent` variants are facts: `AccountUpdated`, `NodeUpserted`,
`NodeHealthChanged`, `ParticipantUpserted`, `ContextUpserted`,
`ContextRemoved`, `MomentAdded`, `MomentUpdated` (lifecycle and/or a payload
of the same kind), `SurfacePatched` (`AppendBlocks`, `ReplaceBlock`,
`ReplaceBlocks`, `AppendText`, `SetActions`, `SetStreaming`; all or nothing),
`ObjectUpserted`, `GrantRequested`, `GrantResolved`, `TaskUpserted`,
`TaskStepChanged` and `SourceStatusChanged` (Connected, Disconnected,
Locked).

## Store

`Store::apply(&mut self, &EventEnvelope) -> Result<Changes, ApplyError>`:

- An envelope with a different `schema` is rejected and changes nothing. Its
  `seq` is not consumed.
- An envelope with `seq <= last_seq` is ignored and returns an empty
  `Changes`. Sequence numbers start at 1.
- An invalid event is rejected and none of it is applied. Examples are an
  unknown id, an illegal lifecycle transition, a payload kind change, a grant
  that was already decided, or a surface or block mismatch. The store records
  the rejection (the last 64 are kept), returns the error and never panics.
  The envelope's `seq` is consumed, so redelivering it is a no-op.
- Application is deterministic. `Changes` lists the changed ids for each
  collection, plus `pin_order` and `source_status` flags, so views can
  notify precisely.
- All other public methods borrow immutably.

The store also tracks the order in which contexts were pinned and the time
each moment entered `NeedsMe` (the envelope `at`).

## Selectors

All selectors are pure functions of `&Store`.

| Selector | Result |
|---|---|
| `context_stack` | Pinned contexts in pin order, then contexts with a `NeedsMe` moment ordered by their oldest one, de-duplicated and capped at 6. It is never sorted by recent activity. |
| `attention` | `AttentionItem { context_id, moment_id, reason, since }` for every `NeedsMe` moment, oldest first. |
| `in_flight` | `InProgress` moments and `TaskProgress` moments whose task is not terminal, ordered by `(at, id)`. |
| `timeline` | The moments of one context, ordered by `(at, id)`. |
| `objects`, `contacts`, `grants(Pending or All)`, `participants(context)` | Dock views. |

## Intents, policy and effects

`Intent { actor: Actor, kind: IntentKind }`. `Actor` is `Human` or
`Agent(ParticipantId)`. The kinds are `SendMessage`, `AttachObject`,
`SurfaceAction`, `Approve { grant_id, confirmed }`, `Deny`, `Reveal`,
`ResolveConflict`, `StartTask`, `CancelTask`, `PinContext`, `UnpinContext`,
`AcceptContact`, `RejectContact` and `Unlock`.

`Unlock` carries a `SecretInput`. This one-shot handle is not `Clone` and not
`Serialize`, and its `Debug` output is `<redacted>`. `Unlock` is excluded from
serde: serializing it returns an error and it cannot be deserialized.

`handle_intent(&Store, Intent) -> Result<Vec<Effect>, IntentError>` checks the
intent against the store and never mutates it.

- `Approve` on a grant that is not pending is an error.
- `Approve` on a `High` or `Critical` grant without `confirmed: true` returns
  `[Effect::RequireConfirmation { intent }]`. The app then shows the
  hold-to-confirm bar and submits a confirmed human intent.
- Agent intents follow the same rules. The agent must be a known `Agent`
  participant. An agent cannot provide human confirmation, so `confirmed` is
  ignored on an agent's high-risk approval. Agents may not unlock.

`Effect` is one of the following:

- `Call { method, body }`. `method` is a contract method name from
  `intent::methods`, such as `grant.approve`. `body` always includes `actor`.
- `RequireConfirmation { intent }`. The app handles this one; it never goes to
  the source.
- `Unlock { account_id, secret }`. The secret only reaches the source.

## UI events and bubbling

`UiEvent { scope_path: Vec<Scope>, kind: UiEventKind }`. The path runs from the
root to the leaf: `Workspace, Context, Moment, Surface, Block`. An element
emits an event with its own scope, and each parent prepends its scope with
`UiEvent::within` before passing the event up.

View-local events such as `ExpandToggled` are consumed below the root (B1).
Events that change business state reach the root, where they become intents
(B2 to B4). A UI event never carries a secret.

## Source

```rust
pub trait Source {
    fn submit(&mut self, effect: Effect);
    fn drain(&mut self, now: Timestamp) -> Vec<EventEnvelope>;
}
```

The trait does not depend on any async runtime. An SDK-backed source can run
its own bridge thread and buffer events until `drain` is called.

## Scenarios (`crates/ramflux-desktop-test`)

`fixtures/scenarios/*.ron` contains one `Scenario` per file:

```text
Scenario { name, description, epoch (optional), timeline: [(offset_ms, DomainEvent)],
           responses: [ResponseRule { on: EffectMatcher, emit: [(delay_ms, DomainEvent)] }] }
EffectMatcher = Call { method, fields } | Unlock { account_id }
```

`MockSource` plays a scenario as a `Source`:

- It assigns `seq` from 1 and sets `at = epoch + offset`.
- It stamps `at` onto every added moment, so fixtures write `at: 0`.
- When a submitted effect matches a rule, it schedules that rule's events
  after the submission time.

`ScenarioPlayer` owns a `Store` and a `MockSource` and provides the following
controls:

- `pause` and `resume`
- `step`, which applies exactly the next event, even while paused
- `advance(ms)`
- `seek(ms)`, which resets the store and replays everything scheduled up to
  that point, including responses triggered earlier
- `dispatch(intent)`, which runs `handle_intent`, submits calls to the source,
  and returns the effects the app must handle

| Scenario | Demonstrates |
|---|---|
| `s01_first_launch` | No daemon, then connect, join a node, create an account, unlock, enter the workspace |
| `s02_daily` | Three contexts, unread counts, a streaming agent reply, an object transfer |
| `s03_intervention` | A high-risk tool request, hold-to-confirm approval, the result in the conversation |
| `s04_safety` | A contact's key changes, a sticky warning, the safety number verified |
| `s05_group_governance` | An invitation, a role change, a member removal with a confirmation bar |
| `s06_in_progress` | A one-sentence request becomes a multi-step task that needs the user midway, then completes |
| `s07_conflict_privacy_handoff` | A CAS conflict, an audited privacy reveal, a cross-device handoff |
| `s08_failures` | A daemon disconnect, an unhealthy node, a failed transfer, an unknown block rendered as fallback |
