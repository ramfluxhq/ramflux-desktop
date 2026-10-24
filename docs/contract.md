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
| `Participant` | `kind` (Human, Agent, Device, Node, Unknown = not reported), `display` (the id when no name is reported), `safety`, `contact`, `safety_number`, `link` (`ContactLink`: None, Accepted, Incoming, Outgoing, Blocked, Removed, Rejected; `link_state()` reads `contact` when no link was reported), `reachable` (`Some(true)` when this account holds a verified device to send to, `None` when not reported), `kind_asserted` (the kind is only the participant's own claim) |
| `ContactCard` | This account's own card as the backend exported it: `text`, optional `link` (`Store::own_contact_card`, from `DomainEvent::OwnContactCardUpdated`) |
| `Context` | `kind` (Direct, Group, Space), `participants`, `pinned`, `unread`, `muted`, `disappearing_secs`, `roles` (Owner, Admin, Member, Bot: exactly the daemon's roles), `epoch` |
| `Moment` | `context_id`, `origin`, `at`, `kind: MomentKind`, `lifecycle` |
| `Object` | `name` and `size` (each `None` when not reported; `label()` shows the id for a missing name), direction, `transfer`, `saved_to`, `preview` |
| `Grant` | scope, `risk`, `single_use`, `args_hash`, expiry, `state` (Pending, Approved, Denied, Expired, Revoked) |
| `StandingApproval` | scope, `max_risk`, expiry, `uses`, `revoked` |
| `AuditEntry` | `at`, `action` (method), `subject` (id), `proposed_by`, `human_confirmed` |
| `Integrations` | read-only MCP servers (`healthy`: `None` while the backend has not checked) and pending A2I events |
| `Task` | intent id, context, `requester` (Actor), `assignee` (participant), `steps`, `state` (Proposed, Running, NeedsMe, Done, Failed, Cancelled), rising `version`, `result`, `result_ref` (surface) |
| `TaskStep` | `id`, `label`, `state` (Pending, Running, NeedsMe, Done, Failed, Skipped), `needs_me`: `Approval { grant_id }` \| `Choice { surface_id }` \| `Input { surface_id }` (required while NeedsMe; kept afterwards as the record) |

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

`Lifecycle::can_transition(from, to)` encodes this table. Moments follow it.
Tasks follow `TaskState::can_transition` (Proposed -> Running | Failed |
Cancelled; Running -> NeedsMe | Done | Failed | Cancelled; NeedsMe -> Running |
Done | Failed | Cancelled) and steps `StepState::can_transition`. A task needs
the requester exactly when one of its steps does; two steps never ask in the
same place; a proposal has not started. Task progress moments follow their
task's state (Proposed and NeedsMe are NeedsMe, Running is InProgress,
Cancelled is Dismissed); `MomentUpdated` lifecycles on them are ignored.

### Versioned surfaces

`DomainEvent::Surface(SurfaceEvent)` carries the surface protocol events:
`Create { moment_id, context_id, origin, surface_id, version: 1, blocks,
actions, fallback_text, hash? }` (fallback required), `Patch { surface_id,
version, base_hash, ops, hash? }` with ops `ReplaceBlock`, `InsertBlock {
index }`, `RemoveBlock`, `SetActions`, `SetFallback`, `StreamAppend {
surface_id, version, block_id, text_delta, final, hash? }` (Text, Markdown and
LogStream blocks) and `Close`. Every change consumes exactly the next version.
A patch whose `base_hash` is not the current hash is not applied and leaves
the surface in conflict until a change against the current version arrives.

A source that reads surfaces from an authority (the SDK source reads the
daemon) sends `Snapshot(SurfaceSnapshot)`: the whole state at a version, which
may skip versions, with `closed`, `out_of_sync` (the authority knows it missed
an author's change: `SurfaceState::OutOfSync`, actions refused) and the blocks
being written (open streams the author is appending to; a block's stream
ends exactly when the authority reports it final). The authority's answers
are `ActionRefused { surface_id, version }` (the surface changed after the
user reviewed it: the store lists the moment in `Changes::surface_refusals`,
the row asks for a new review) and `UpdateRequested { surface_id, refused? }`
(the answer to the user's request for the author's current state, kept as
`SurfaceRecord::update`; a refusal is the text key of the authority's typed
refusal code). `UpdatedAt { surface_id, at }` records the author's
last-updated time (`SurfaceRecord::updated_at`, display only; the moment's
`at` is the author's posting time), and `Removed { surface_id }` removes the
moment of a surface the authority no longer holds.

Hashes come from the source: the surface protocol's snapshot hash, which the
daemon computes. The desktop has no hash encoder. A version without a source
hash (fixtures, unversioned payloads) is labelled `v<version>`
(`SurfaceHash::of_version`), which binds actions to the version number only;
fixtures name their patch bases by these labels.

Limits are enforced at the reducer edge and match the protocol (64 blocks, 16
actions, 16,384 characters per string, a 16,384-byte state, 4,096-byte text
deltas, 16,384 characters streamed per block): a surface or change over a
limit leaves only the fallback text (`SurfaceState::OverLimit`), never a
partial rendering, and refuses further changes except `Close`. A closed
surface is a frozen record. The store keeps a `SurfaceRecord` (version, hash,
state, streams, update request, last-updated time) per surface. `export::timeline_text` renders any conversation as plain text
(surfaces as their fallback text), the terminal degradation.

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
`Surface` (versioned surface events, see above), `ObjectUpserted`,
`GrantRequested`, `GrantResolved`, `TaskUpserted` (full update, rising
version), `TaskStepChanged` (one step, rising version), `TaskStateChanged`
(state only; ending a task skips its unfinished steps), `SourceStatusChanged` (Connected, Disconnected,
Locked), `ContextSettingsChanged` (pin, mute, disappearing timer),
`AccountLockChanged`, `NodeSettingsChanged` (enabled; setting a default clears
the previous one), `NodeProbed` (also sets health), `NodeRemoved`,
`DeviceUpserted`, `GrantRevoked` (Approved to Revoked only),
`StandingApprovalUpserted`, `AuditAppended` (the last 512 are kept),
`IntegrationsUpdated`, `CapabilitiesUpdated` (contract methods the backend
cannot perform, each with the reason shown to the user), `DaemonInfoUpdated`
(socket path, open accounts), `SyncStatusChanged` (how the source is kept up
to date: `Connecting`, `Stream`, `Polling` or `Off`, why the stream is not
used, the stream cursor, and the daemon's background sync state, last run and
unrouted count), `EffectFailed` (method, code
`UnsupportedByBackend` / `Daemon(code)` / `Transport` / `Timeout`, message;
the last 32 are kept; no domain entity changes) and `ListingsLoaded` (the
source answered these collections in full: `Nodes`, `Accounts`, `Contexts`,
`Contacts`, `Objects`, `Approvals`).

An empty collection is either still loading or listed and empty. The store
records each `ListingsLoaded` (`Store::is_listed`) and `ListState::of(listed,
empty)` gives `Loading`, `Empty` or `Ready`; every list view shows a spinner
only while loading and an explicit empty state once listed.

Policy refuses an intent whose method the backend declared unsupported
(`IntentError::UnsupportedByBackend`), before any hold, and the UI shows such
controls disabled with the reason. A method may be unsupported only in some
context kinds (`methods::in_context`, e.g. `message.send@Space`,
`object.send@Group`). Sources never fake data for a method they lack. A
reason or failure message is a locale key (`backend.*`), optionally followed
by one space and a JSON object of string arguments the UI substitutes.

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
  collection, plus `pin_order`, `listings` and `source_status` flags, so
  views can notify precisely.
- All other public methods borrow immutably.

The store also tracks the order in which contexts were pinned and the time
each moment entered `NeedsMe` (the envelope `at`).

Moment rows hold metadata only. A message's text moves into a bounded,
memory-only body cache (`Store::body`, `MAX_BODIES` = 4 096 per store; the
least recently loaded body is evicted first; recency moves only on apply, so
replay stays deterministic). An account that locks drops every body
(`Changes::bodies_cleared`). A view that shows a message whose body is not in
memory asks the source again (`Source::request_bodies`); the body comes back
as a `MomentUpdated` carrying the message payload.

The reducer maintains secondary indexes with the collections they index (a
timeline per context, `NeedsMe` and `InProgress` sets, the contexts of each
participant, the moments presenting a grant or a task, the tasks waiting on a
grant), so every selector costs O(k) in the size of its result.

## Selectors

All selectors are pure functions of `&Store`.

| Selector | Result |
|---|---|
| `context_stack` | Pinned contexts in pin order, then contexts with a `NeedsMe` moment ordered by their oldest one, de-duplicated and capped at 6. It is never sorted by recent activity. |
| `attention` | `AttentionItem { context_id, moment_id, reason, since }` for every `NeedsMe` moment, oldest first. An intervention whose grant a task step owns is left out: the task moment carries it (one moment, one place). |
| `owning_task_step` | `TaskStepRef { moment_id, task_id, step }` of the task step waiting on a grant. |
| `in_flight` | `InProgress` moments and `TaskProgress` moments whose task is not terminal, ordered by `(at, id)`. |
| `timeline` | The moments of one context, ordered by `(at, id)`. |
| `needs_me_count(context)`, `contact_requests` | Attention per context; open contact requests. |
| `incoming_contact_requests`, `blocked_contacts` | Participants whose contact link is an incoming request, or blocked. |
| `presenting_moment(grant)` | The intervention, approval block or owning task step that presents a grant. |
| `contexts_with(participant)` | The contexts a participant is a member of. |
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
  `MuteContext`, `SetDisappearing`.
- Tasks: `StartTask { context_id, assignee, text }` (an agent member of the
  context; body `{conversation_id, assignee, text, constraints, proposal_id}`),
  `AcceptTaskProposal` (human only; `task.start` with `proposal_id`),
  `CancelTask` (only the requester; also declines a proposal).
  `SurfaceAction` carries the `version` and `surface_hash` it was shown on
  (the source hashes its arguments with the protocol's argument hash); a
  closed, out-of-date, out-of-sync, over-limit or changed surface refuses it.
  `RequestSurfaceUpdate` (human only) asks the author of an out-of-sync
  surface for its current state (`surface.request_resync`; the daemon
  rate-limits it and only the author answers).
- Moments: `SurfaceAction`, `RequestSurfaceUpdate`, `Approve`, `Deny`, `Reveal`, `ResolveConflict`,
  `AcceptContact`, `RejectContact`, `VerifySafetyNumber`, `AcceptHandoff`,
  `DeclineHandoff`.
- Contacts: `AddContact { participant_id }` (the id the contact shared; human
  only), `Contact { participant_id, action }` with `ContactAction` Accept,
  Reject (incoming link), Block (accepted, incoming or outgoing), Unblock
  (blocked), Remove (accepted or blocked). Methods `contact.add`,
  `contact.accept`, `contact.reject`, `contact.block`, `contact.unblock`,
  `contact.remove`; `contact.request` names sending a request to the
  contact's node. `check_add_contact` is the add check the form also runs.
  `AddContactCard { card }` (human only; method `contact.card.import`) adds
  the contact a pasted or imported card names; `ExportContactCard` (human
  only; `contact.card.export`) asks for this account's own card.
  `classify_contact_input` tells a card (a link with a scheme, several lines,
  or longer than any id) from a principal id; `check_add_contact_card` is the
  card check. A start is refused with `NotReachable` while the contact is
  reported unreachable.
- Groups: `CreateGroup { context_id, title, members }` (human only; method
  `group.create`; a fresh conversation id, a name, and accepted contacts a
  conversation could start with now, each once; `check_create_group`) and
  `RenameContext { context_id, title }` (groups only; `conversation.rename`;
  the name this account keeps).
- Objects: `PullObject` (queued inbound only), `SaveObject` (complete only),
  `RetryTransfer` (failed only).
- Governance and approvals: `Governance { context_id, op }` (`AddMember`,
  `RemoveMember`, `Kick`, `Ban`, `SetRole`, `CreateInvite { participant_id,
  role }`: an invitation names the invited participant), `RevokeGrant`,
  `RevokeStandingApproval`.
- Entry: `Unlock`, `CreateAccount { kind, label, node_id, secret }`,
  `ActivateDevice { node_id, device_label, code }`, `RestoreBackup { path,
  secret }`, `AddNode { url }`, `ConfirmNode { node_id, pin_source }`.
- Settings: `LockAccount`, `RotatePassphrase { current, next }`,
  `ExportBackup { account_id, path, secret }`, `RevokeDevice`, `SyncDevices`, `ProbeNode`, `RemoveNode`,
  `SetDefaultNode`, `SetNodeEnabled`, `ExportDiagnostics`.

Kinds that carry a `SecretInput` (unlock, create, activate, restore, backup
export, rotate) are excluded from serde. A `SecretInput` is not `Clone`, not `Serialize`, its
`Debug` output is `<redacted>`, and its buffer is zeroized on drop.

`handle_intent(&Store, Intent, now: Timestamp) -> Result<Vec<Effect>, IntentError>`
checks the intent against the store at time `now` and never mutates it.

- The attended human's hold is required (without `confirmed_by` the result is
  `[Effect::RequireConfirmation { intent }]`) for: approving a `High` or
  `Critical` grant, a `High` or `Critical` surface action, `Kick`, `Ban` and
  `SetRole`, every `Reveal`, `RevokeGrant`, `RevokeStandingApproval`,
  `RevokeDevice`, blocking or removing a contact, and `ConfirmNode` with a
  trust-on-first-use (`Manual`) pin.
- Grants that are not pending, or whose expiry is at or before `now`, cannot
  be approved or denied; only approved grants can be revoked.
- `ConfirmNode` must match the discovered pin source; new accounts and new
  devices bind only to confirmed nodes; a node with a bound account cannot be
  removed; the default node cannot be disabled.
- Agent proposals follow the same rules; the agent must be a known `Agent`
  participant. Agents may never unlock, create, activate or restore an
  account, rotate its passphrase, export a backup, sync or revoke devices,
  confirm a node, verify a safety number, answer a handoff, add a contact,
  share the contact card, start a conversation, or create a group.

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
    fn focus(&mut self, _context: Option<&ContextId>) {}
    fn request_bodies(&mut self, _moments: &[MomentId]) {}
}
```

The trait does not depend on any async runtime. An SDK-backed source
(`crates/ramflux-desktop-sdk`) runs its own bridge thread and buffers events
until `drain` is called. `focus` tells a pulling source which context the user
looks at, so it can keep it fresh until the daemon pushes events itself.
`request_bodies` asks a source to deliver evicted message bodies again; the
mock source re-delivers the newest payload it already delivered, the SDK
source re-reads the context (a group's text is known only while its
decrypted copy is kept, bounded as well).

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
| `s10_ai_participant` | Demo 3: ask the AI participant in a group, streamed reply, task proposal, needs-me approval with a hold, result patch, done (deny and cancel branches) |
| `s11_surface_versions` | A choice answered in a surface, a change during review, a conflicting patch and its resend, close |
| `s12_surface_limits` | A surface over the action limit, a stream over the delta limit |
