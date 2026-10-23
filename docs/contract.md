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
| R6 | Humans and agents share one pipeline. Every `Intent` records who proposed it (`proposed_by: Actor`) and, after a hold, that the human confirmed it (`confirmed_by`). |

## Identifiers and time

`AccountId`, `NodeId`, `DeviceId`, `ParticipantId`, `ContextId`, `MomentId`,
`ObjectId`, `GrantId`, `StandingApprovalId`, `TaskId`, `SurfaceId` and
`BlockId` are string newtypes that
serialize as plain strings. `Timestamp(i64)` holds unix milliseconds and also
serializes as a plain number. Core never reads the clock. Every time value
comes from a source.

## Model

The store keeps each entity in its own collection, keyed by id:

| Entity | Key fields |
|---|---|
| `Account` | label, `kind` (HumanInteractive, HeadlessAi), node, device, `lock_state`, `self_participant_id` (the participant that is this account; outbound moments and receipts key on it, never guessed) |
| `Node` | NODE-01 profile: endpoints, `fingerprint`, `pin_source` (Bootstrap, Invite, Manual = trust on first use), `health`, `capabilities`, `expires_at`, `confirmed`, `builtin`, `enabled`, `is_default`, last `probe` (gateway and relay outcomes) |
| `Device` | account, label, `this_device`, `revoked`, `last_seen` |
| `Participant` | `kind` (Human, Agent, Device, Node), `display`, `safety`, `contact`, `safety_number` |
| `Context` | `kind` (Direct, Group, Space), `participants`, `pinned`, `unread`, `muted`, `disappearing_secs`, `roles`, `epoch` |
| `Moment` | `context_id`, `origin`, `at`, `kind: MomentKind`, `lifecycle` |
| `Object` | name, size, direction, `transfer`, `saved_to`, `preview` |
| `Grant` | scope, `risk`, `single_use`, `args_hash`, expiry, `state` (Pending, Approved, Denied, Expired, Revoked) |
| `StandingApproval` | scope, `max_risk`, expiry, `uses`, `revoked` |
| `AuditEntry` | `at`, `action` (method), `subject` (id), `proposed_by`, `human_confirmed` |
| `Integrations` | read-only MCP servers and pending A2I events |
| `Task` | intent id, context, `actor`, `steps` (each may wait on a `grant_id`), `lifecycle`, `result` |

Messages carry an optional `attachment` and a `franking` commitment; conflict
sides carry the conflicting `lines` for a side-by-side compare.

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
`TaskStepChanged`, `SourceStatusChanged` (Connected, Disconnected,
Locked), `ContextSettingsChanged` (pin, mute, disappearing timer),
`AccountLockChanged`, `NodeSettingsChanged` (enabled; setting a default clears
the previous one), `NodeProbed` (also sets health), `NodeRemoved`,
`DeviceUpserted`, `GrantRevoked` (Approved to Revoked only),
`StandingApprovalUpserted`, `AuditAppended` (the last 512 are kept) and
`IntegrationsUpdated`.

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
| `attention` | `AttentionItem { context_id, moment_id, reason, since }` for every `NeedsMe` moment, oldest first. An intervention whose grant a task step owns is left out: the task moment carries it (one moment, one place). |
| `owning_task_step` | `TaskStepRef { moment_id, task_id, step }` of the task step waiting on a grant. |
| `in_flight` | `InProgress` moments and `TaskProgress` moments whose task is not terminal, ordered by `(at, id)`. |
| `timeline` | The moments of one context, ordered by `(at, id)`. |
| `objects`, `contacts`, `grants(Pending or All)`, `participants(context)` | Dock views. |

## Intents, policy and effects

`Intent { proposed_by: Actor, confirmed_by: Option<HumanConfirmation>, kind: IntentKind }`.
`Actor` is `Human` or `Agent(ParticipantId)`. `HumanConfirmation` (gesture
`Hold`, `held_ms`) is an unforgeable capability: it has no public constructor,
is neither `Clone` nor `Deserialize`, and only a `ConfirmationIssuer` mints it,
for a completed hold of at least `MIN_HOLD_MS`. The application creates one
issuer and the intent root (owner of the hold-to-confirm strip) keeps it.
`confirmed_by` is skipped on deserialization, so a stored, received or scenario
intent is always unconfirmed, and policy counts a confirmation only when it is
present with a full hold. Confirming never changes `proposed_by`, so an agent
proposal confirmed by the human keeps the agent as proposer. Agent intents are
built without a confirmation (`Intent::agent`) and no UI event can carry one.

The kinds, by area:

- Conversation: `SendMessage`, `AttachObject`, `PinContext`, `UnpinContext`,
  `MuteContext`, `SetDisappearing`, `StartTask`, `CancelTask`.
- Moments: `SurfaceAction`, `Approve`, `Deny`, `Reveal`, `ResolveConflict`,
  `AcceptContact`, `RejectContact`, `VerifySafetyNumber`, `AcceptHandoff`,
  `DeclineHandoff`.
- Objects: `PullObject` (queued inbound only), `SaveObject` (complete only),
  `RetryTransfer` (failed only).
- Governance and approvals: `Governance { context_id, op }` (`AddMember`,
  `RemoveMember`, `Kick`, `Ban`, `SetRole`, `CreateInvite`), `RevokeGrant`,
  `RevokeStandingApproval`.
- Entry: `Unlock`, `CreateAccount { kind, label, node_id, secret }`,
  `ActivateDevice { node_id, device_label, code }`, `RestoreBackup { path,
  secret }`, `AddNode { url }`, `ConfirmNode { node_id, pin_source }`.
- Settings: `LockAccount`, `RotatePassphrase { current, next }`,
  `ExportBackup`, `RevokeDevice`, `SyncDevices`, `ProbeNode`, `RemoveNode`,
  `SetDefaultNode`, `SetNodeEnabled`, `ExportDiagnostics`.

Kinds that carry a `SecretInput` (unlock, create, activate, restore, rotate)
are excluded from serde. A `SecretInput` is not `Clone`, not `Serialize`, its
`Debug` output is `<redacted>`, and its buffer is zeroized on drop.

`handle_intent(&Store, Intent, now: Timestamp) -> Result<Vec<Effect>, IntentError>`
checks the intent against the store at time `now` and never mutates it.

- The attended human's hold is required (without `confirmed_by` the result is
  `[Effect::RequireConfirmation { intent }]`) for: approving a `High` or
  `Critical` grant, a `High` or `Critical` surface action, `Kick`, `Ban` and
  `SetRole`, every `Reveal`, `RevokeGrant`, `RevokeStandingApproval`,
  `RevokeDevice`, and `ConfirmNode` with a trust-on-first-use (`Manual`) pin.
- Grants that are not pending, or whose expiry is at or before `now`, cannot
  be approved or denied; only approved grants can be revoked.
- `ConfirmNode` must match the discovered pin source; new accounts and new
  devices bind only to confirmed nodes; a node with a bound account cannot be
  removed; the default node cannot be disabled.
- Agent proposals follow the same rules; the agent must be a known `Agent`
  participant. Agents may never unlock, create, activate or restore an
  account, rotate its passphrase, export a backup, sync or revoke devices,
  confirm a node, verify a safety number, or answer a handoff.

`Effect` is one of the following:

- `Call { method, body }`. `method` is a contract method name from
  `intent::methods`, such as `grant.approve`. Every body includes
  `proposed_by` and `confirmed_by` (null or the hold record): the audit pair.
- `SecretCall { method, body, secrets }`. The body never contains secret
  material; the secrets travel beside it and only reach the source.
- `RequireConfirmation { intent }`. The app handles this one; it never goes to
  the source.

## UI events and bubbling

`UiEvent { scope_path: Vec<Scope>, kind: UiEventKind }`. The path runs from the
root to the leaf: `Workspace, Context, Moment, Surface, Block`, with a
`Panel(name)` level for pages, docks and overlays (`entry`,
`settings.nodes`, `dock.members`, `palette`). An element
emits an event with its own scope, and each parent prepends its scope with
`UiEvent::within` before passing the event up.

View-local events (`ExpandToggled`, `FieldChanged`, `FocusMomentRequested`) are consumed below the
root (B1); a level may add context it owns (the moment level adds collected
form values to `ActionInvoked`, B3).
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
Scenario.companions = [Companion { account_id, scenario }]   (background accounts)
```

`MockSource` plays a scenario as a `Source`:

- It assigns `seq` from 1 and sets `at = epoch + offset`.
- An added moment written with `at: 0` is stamped with the delivery time; any
  other `at` is a signed offset from the epoch (history such as yesterday).
  Reveal records and node probes written with `at: 0` are stamped too.
- Effects no rule answers fall back to the built-in local echo (pin, unpin,
  mute, disappearing timer, default, enabled and removed nodes, account lock),
  so local intents round-trip in every scenario. Every call confirmed with a
  hold is also echoed as an `AuditAppended` record.
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
| `s01_first_launch` | No daemon, then bootstrap and invite nodes, a node added by URL (TOFU, hold), create, join or restore an account, lock and unlock |
| `s02_daily` | Three contexts, unread counts, a streaming agent reply, an object transfer |
| `s03_intervention` | A high-risk tool request, hold-to-confirm approval, the result in the conversation |
| `s04_safety` | A contact's key changes, a sticky warning, the safety number verified |
| `s05_group_governance` | An invitation, a role change, a member removal with a confirmation bar |
| `s06_in_progress` | A one-sentence request becomes a multi-step task that needs the user midway, then completes |
| `s07_conflict_privacy_handoff` | A CAS conflict, an audited privacy reveal, a cross-device handoff |
| `s08_failures` | A daemon disconnect, an unhealthy node, a failed transfer and its retry, an unknown block rendered as fallback |
| `s09_multi_account_day` | Two accounts (companion `s09_work_account`), attention in the background account, switch and resolve; node, device and integration data for settings |
