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
| `Participant` | `kind` (Human, Agent, Device, Node, Unknown = not reported), `display` (the id when no name is reported), `safety` (`SafetyState`: Unverified, Verified, KeyChanged, Forked, VerificationStale = verified before the authority changed, verify again), `contact`, `safety_number`, `link` (`ContactLink`: None, Accepted, Incoming, Outgoing, Blocked, Removed, Rejected; `link_state()` reads `contact` when no link was reported), `reachable` (`Some(true)` when this account holds a verified device to send to, `None` when not reported), `kind_claim` (`KindClaim`: None = nothing reported, the safe default; Asserted(`PeerKind` Human / HeadlessAi / Bot) = only the participant's own claim; Verified(kind) = a root-signed public profile states it), `name_warnings` (`NameWarnings`: `mixed_script`, `confusable_with` another principal; advisory only), `root_source` (`RootSource`: Lookup = provisional name lookup, Card = pinned by their card, Verified = safety numbers compared; `None` when not reported) |
| `ContactCard` | This account's own card as the backend exported it: `text`, optional `link` (`Store::own_contact_card`, from `DomainEvent::OwnContactCardUpdated`) |
| `Context` | `kind` (Direct, Group, Space), `participants`, `pinned`, `unread`, `muted`, `disappearing_secs`, `roles` (Owner, Admin, Member, Bot: exactly the daemon's roles), `epoch` |
| `Moment` | `context_id`, `origin`, `at`, `kind: MomentKind` (a `TaskEvent` carries `actor_kind_verified`; a `NameChange` is an authenticated public rename: participant, old and new name), `lifecycle` |
| `Object` | `name` and `size` (each `None` when not reported; `label()` shows the id for a missing name), direction, `transfer`, `saved_to`, `preview` |
| `Grant` | scope, `risk`, `single_use`, `args_hash`, expiry, `state` (Pending, Approved, Denied, Expired, Revoked) |
| `StandingApproval` | scope, `max_risk`, expiry, `uses`, `revoked` |
| `AuditEntry` | `at`, `action` (method), `subject` (id), `proposed_by`, `human_confirmed` |
| `Integrations` | read-only MCP servers (`healthy`: `None` while the backend has not checked) and pending A2I events |
| `Task` | intent id, context, `requester` (Actor), `assignee` (participant), `steps`, `state` (Proposed, Running, NeedsMe, Done, Failed, Cancelled), rising `version`, `result`, `result_ref` (surface), `flags` (stalled, compacted), `actors` (`TaskActors`: requester / assignee / acceptor, each a `TaskActor` with participant, `device_id` of its last event, profile `kind` and `kind_verified` for its role; `assignee_kind_verified()`), `answer` |
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
(state only; ending a task skips its unfinished steps), `TaskSnapshot` (the
whole task at its authority's version: versions may skip, the same version
changes only flags and step places, a proposal gets its requester when
accepted, a finished task is frozen), `TaskAnswered` (the source's answer to
this desktop's last request about a task: a decision pending confirmation, a
decision the assignee has not confirmed in time (`DecisionUnconfirmed`), a
decision on an approval that could not be delivered to the assignee
(`DecisionUndelivered { step_id, grant_id, approved }`, retried unchanged),
or a typed refusal `Changed`, `Finished`, `NotAllowed`, `AlreadyDecided`,
`TooLarge`, `DecisionRejected`, `Stalled`, `Busy`, `QuotaExhausted`,
`Unreachable` (retryable), `Unauthenticated`, `Untrusted` (neither
retried), `Other`; kept until the version rises), `TaskRemoved` (the source
pruned the task: it and its moments are dropped),
`SourceStatusChanged` (Connected, Disconnected,
Locked), `ContextSettingsChanged` (pin, mute, disappearing timer),
`AccountLockChanged`, `NodeSettingsChanged` (enabled; setting a default clears
the previous one), `NodeProbed` (also sets health), `NodeRemoved`,
`DeviceUpserted`, `GrantRevoked` (Approved to Revoked only),
`StandingApprovalUpserted`, `AuditAppended` (the last 512 are kept), `BackupJobUpdated` (a full
backup or restore job, whole, rising version),
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
  `AcceptTaskProposal` (human only; `task.start` with `proposal_id` and the
  `version` shown, which the SDK source sends as the daemon's only
  acceptance, `task.accept {task_id, version}`), `CancelTask` (only the
  requester; body adds `conversation_id` and `decline: true` for a
  proposal), `RetryTaskDecision` (only the requester, only while the task's
  answer is `DecisionUndelivered`: `grant.approve` / `grant.deny` with
  `retry: true` for the same approval, no new confirmation and no second
  `task.resolve`; the daemon resends the same signed decision). A task's flags say whether its assignee is stalled, its
  history compacted and the assignee's kind verified (`Actor::Member` is
  another member who asked for a task); a finished task's `result` is the
  assignee's signed summary. Approving or denying a grant a task step waits
  on is the requester's only; the SDK source then sends `task.resolve` for
  that step with the decision id the grant flow answered. A choice or input
  step is answered in the assignee's surface (an action whose values carry
  `decision: "resume"` or `"deny"`, and whose signed bytes carry the step it
  answers, `taskBinding {task_id, step_id}`); a surface read that lists this
  account's bound action (its own retained outgoing action, or the author's
  receipt) resolves the step the binding names with that row's
  `action_receipt_id`, also after a restart. A pending resolution is shown on
  the step `pending_resolution.step_id` names. Task refusals are read from
  the daemon's typed codes, never its text; the transport codes are
  `Unavailable` (retryable), `Unauthenticated` and `TransportSecurity` (not
  retried; shown as such, never as a lost connection).
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
  `AddContactCard { card, replace_provisional_root }` (human only; method
  `contact.card.import`, the daemon's `contact.add {card}`) adds the contact a
  pasted or imported card names; `replace_provisional_root` is set only after
  the user compared both roots of a `root_replacement_confirmation_required`
  refusal and confirmed the card replaces the looked-up root; `ExportContactCard` (human
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
- Backup: `CreateBackup { account_id, path, secret,
  allow_missing_attachments }`, `VerifyRestore { path, secret }`,
  `InstallRestore { job_id }`, `CancelBackupJob { job_id }` (see "Full
  backup and restore").

Kinds that carry a `SecretInput` (unlock, create, activate, restore, backup
export, full backup, restore verification, rotate) are excluded from serde. A `SecretInput` is not `Clone`, not `Serialize`, its
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
| `s13_born_v3` | A fresh device whose daemon offers account creation with a recovery phrase (after two seconds); the mock source answers the ceremony |

## Voice calls (DESK-F16, on CALL-01)

One-to-one voice calls in direct conversations, on the daemon's call API
(CALL-01, product f55c7e0; the fake daemon `ramflux-desktop-test`
`fake_calls` follows it). Each account snapshot reads `call.list`: a daemon
that answers it has the call API, and the call methods are declared
supported. Until then, with a daemon without it, or with calls turned off
(`SdkConfig::call_api` false; `RAMFLUX_DESKTOP_CALLS=0` in the application)
the call methods are declared unsupported (`backend.gap.calls`) and the call
button is disabled with that reason.

**Store.** A `Call` (conversation, peer, caller, direction, `CallState`:
`OutgoingRinging`, `IncomingRinging`, `Connecting`, `Active`,
`Ended(CallEndReason)` with `Completed`, `Cancelled`, `Declined`, `Busy`,
`Timeout`, `AnsweredElsewhere`, `Failed`, `Missed`, the daemon's end reasons
from this account's side; the daemon's rising per-call `version`; placed,
answered and ended times) arrives whole as `DomainEvent::CallUpserted`. The
reducer refuses a version that does not rise, a changed conversation, peer,
caller or direction, and an illegal transition (ringing → connecting or
ended, connecting → active or ended, active → ended). Each call keeps one
derived `MomentKind::Call` record (`call:<call id>`) in its conversation whose
lifecycle follows the call (a ringing incoming call needs the user; an ended
one is done). `Call::outcome` reads an ended call from this account's side:
answered (duration), missed, declined, no answer, busy, cancelled, answered
elsewhere, failed. Selectors: `current_call`, `ringing_calls`,
`call_under_way`.

**Intents.** `StartCall { context_id }` (human only; a direct conversation;
the peer not in a changed key state nor unreachable; one call at a time),
`AnswerCall` (a call ringing here, no other call under way), `RejectCall`,
`HangUpCall` (not ended): methods `call.invite`, `call.answer`,
`call.reject`, `call.hangup`. Agents never take part in calls.

**Daemon API (CALL-01 r2).** `call.invite {conversation_id}`,
`call.answer|reject|hangup {call_id}`, each answering with the call row;
event `call.changed` with the call row: `call_id`, `conversation_id`,
`peer_id`, `direction` (`outgoing`/`incoming`), `state`
(`outgoing_ringing`, `incoming_ringing`, `connecting`, `active`, `ended`),
`end_reason` (`completed`, `cancelled`, `declined`, `busy`, `timeout`,
`answered_elsewhere`, `failed`, `missed`), `seq`, `created_at`,
`answered_at` (when it became active), `ended_at` (Unix seconds),
`media_generation` (0, not read). Call ids are base64url and never start
with `-`. `connecting` → `active` is the daemon's: signaling done and this
side's relay allocation held. `call.list {}` answers `{calls: [rows]}` (at
most 128, ringing, active and recent), read with every snapshot so calls
come back after a reload; `message.list` rows of `kind: "call"` carry the
same row as `call` and are read as the call, never as message text.
`call.media.session {call_id}` (only while `active`, once per call) answers `relay.endpoint` (`host:port`),
`relay.token` (base64url, opaque), `relay.lease_id` (base64url, decoded 16 bytes),
`relay.side` (0 caller, 1 callee), `relay.rebind_key` (base64url, decoded 32 bytes),
`sframe.cipher_suite` (IANA id) and
`sframe.send` / `sframe.receive` `{kid, base_key}` (base64url).

**Media session.** Secret, so never an event: once the call is active the
store handle asks the source once per call (`Source::request_call_media`,
`take_call_media`); the SDK hands the parsed `CallMedia` (token, rebind
credential `CallRebind`, keys; wiped on drop, never printed) straight to the
engine. KIDs are 0 caller → callee and 1 back. The engine consumes its
`MediaSession` (not `Clone`); the sending key can be taken by one sender
only, so a key's counter starts once at zero and is never reused: audio
that fails, or a lost reply, is not started again on the same call (the
daemon refuses a second handout; a fresh call brings fresh keys). The audio
stays with its call while another account is shown.

**Media (`ramflux-desktop-media`).** Opus 48 kHz mono 20 ms (libopus, VoIP,
in-band FEC) in a minimal RTP packet (V=2, PT 111, seq, timestamp, SSRC)
whose payload is SFrame (RFC 9605, AES_256_GCM_SHA512_128; the RTP header is
the authenticated metadata; per-key replay window). Towards the relay each
datagram is `u32be(token length) || token || packet`; the relay forwards the
packet alone; packets from any other address are dropped. A rebinding
challenge from the relay is answered at once from the same socket (below;
`ramflux_desktop_media::rebind`), and the SFrame sender and receiver go on
unchanged. Adaptive jitter
buffer (target from RFC 3550 jitter, FEC or concealment for a lost frame).
No echo cancellation yet (webrtc-audio-processing needs meson): calls are
headset-only, and the in-call bar says so. On macOS the bundle carries
`NSMicrophoneUsageDescription`; a denied microphone delivers silence, which
the engine reports and the bar explains with a shortcut to System Settings.

**CALL-01 r2 secret handoff and consent.** Each participant receives only its own
relay token and rebind key over its authenticated gateway → daemon → media-engine
channel. Neither secret travels through the peer. The caller sends a public lease
offer; the callee signs consent binding call ID, both root-qualified endpoints,
lease ID and hard expiry, then claims its own credentials through its gateway.
The callee becomes active after authenticating that offer and obtaining its private
allocation. The caller becomes active after authenticating the answer and allocating
its own side. The gateway checks current root-signed active device keys and epochs.
The node charges only the caller until consent, and the callee once after verified
consent. There is a four-hour hard lease limit; ordinary authorization is 60 seconds
and the caller renews every 20 seconds.

`call.media.session` hands out keys once per call. Every SFrame key starts exactly
once at counter zero; it cannot be reacquired after a media-engine restart or lost
reply. A fresh call is required. Daemon restart, or device/key change, ends the call
with `end_reason: failed`; live endpoint handoff/rekey is deferred. Tokens, rebind
keys and media keys must never enter public state, events, timeline rows or logs.

**CALL-01 r2 NAT rebinding contract (media-engine implementation required).**
Ordinary media stays source-address pinned, with no per-packet MAC. Keepalives are
empty token-framed payloads every five seconds and do not renew authorization.
The connected media socket accepts a raw challenge only from its configured relay:

```text
challenge = ASCII("RFVC") || cookie[32] || new_addr_bytes
new_addr_bytes = family_u8 || IP_octets || UDP_port_u16be
family = 4 with 4 IPv4 octets, or 6 with 16 IPv6 octets
```

The relay provides the public NAT-mapped address it observed. Use those bytes, not
the engine's local socket address. Challenge length is 43 bytes for IPv4 or 55 for
IPv6. Within five seconds, compute and send from that socket:

```text
tag = HMAC-SHA256(decoded_rebind_key,
  ASCII("ramflux.voice.rebind.v1") || decoded_lease_id[16] || side_u8 ||
  cookie[32] || new_addr_bytes)[0..16]
response_payload = ASCII("RFVE") || cookie[32] || tag[16]
datagram = token_length_u32be || decoded_token || response_payload
```

Only a valid address/lease/side-bound HMAC echo moves the pin. A copied token and
cookie alone cannot authorize a change. The relay consumes this control payload;
it does not forward it as media. Each side has one outstanding five-second
challenge and shares its normal bandwidth budget. Changing the UDP socket alone
must preserve the SFrame sender counters and receiver replay window. Recreating
the SFrame engine with the same keys is forbidden. Public `media_generation` stays
zero; live handoff with incremented KIDs belongs to a later contract.

The backend rebinding helper has independent known-answer tests. For an HMAC key
of byte values 0..31, decoded lease ID 0..15, side 1 and cookie 32..63, expected
16-byte tags are `5a2ff8f9d9c805d3a158c94d51bb0bbe` for IPv4 `203.0.113.7:54321`
and `07d911fa05814f8c69af12f9420b5aee` for IPv6 `[2001:db8::1234]:54321`.
These are public test vectors, not deployment keys.

## Contact requests (DESK-F21, on CONTACT-02)

Adding a contact sends the peer a request; until the peer accepts, messages
from the requester are held by the peer's daemon (a "message request") and
delivered into the conversation on accept, dropped on decline or block.
Aligned with the CONTACT-02 contract as merged on product main (pinned
`6158459`, `docs/contact_cards.md` "Contact consent and message requests",
r4). The fake daemon (`ramflux-desktop-test` `fake_contact_requests`)
follows the same contract.

**Store.** A `ContactRequest` (daemon `id`, `peer`, `RequestDirection`
`Incoming` / `Outgoing`, `RequestState` `Pending`, `Accepted`, `Declined`,
`Blocked`, `Expired`, the requester's `note`, the `safety_number` to compare,
`held_messages` and a `preview` of the first, `expires_at`), one per peer and
direction. `ContactRequestsListed` replaces the pending incoming requests (an
answered one stays, so its end can be told); `ContactRequestUpserted` replaces
one request (either direction), except that an update of the same request
without the private fields (note, preview, safety number, expiry: what an
event carries) keeps the ones the last listing gave. Locking the account
drops every note, preview and safety number, as it drops message bodies.
`ContactRequestPolicyUpdated` sets the account's `RequestPolicy` (`Manual` /
`AutoAccept`). Selectors: `pending_contact_requests` (incoming, waiting, by
name), `outgoing_contact_request(peer)` (this account's own request until
accepted).

**Intents.** `AnswerContactRequest { request_id, answer: Accept | Decline |
Block }` (human only; a pending incoming request): methods
`contact.request.accept`, `contact.request.decline`, `contact.request.block`;
Block drops whatever the requester sends from now on and needs the hold.
`SetContactRequestPolicy { policy }` (human only): `contact.requests.policy.set`.
`AddContact` on a participant with a pending incoming request is refused
(`ContactLinkState Incoming`): the request is answered instead. `AddContact`
on an accepted contact is refused, except when this account's own request to
them is not accepted (pending, expired or declined): that asks again,
explicitly. UI events: `ContactRequestAnswered { request_id, answer }`,
`ContactRequestPolicySwitched { auto_accept }`; Send again on a contact row
emits `AddContactRequested`.

**Daemon API (CONTACT-02, product main).**

- `contact.add {principal_id}` (and `contact.card.import`) sends the request;
  the answer's `contact` row and every `contact.list` row carry
  `request_state` (`pending`, `accepted`, `declined`, `expired`, or null) for
  this account's own request to that contact, and `request_id`; the state
  stays on the contact row after the request history is pruned. Adding again
  while the request is pending resends the same logical request (same id;
  explicit retry); an expired request, or a declined one after its 24 h
  cooldown, is asked anew. An accepted contact is refused `already_contact`.
  Within the cooldown the daemon refuses the add untyped (`ValidationFailed`,
  no `reason_code`), also after the declined row was pruned from the list
  (the daemon keeps the declined status, and the contact row still reports
  it); the desktop, knowing the request was declined, says so plainly
  (`backend.contact.request_cooldown`: they declined recently, ask again a
  day after). This is explained for an add by id; a re-request by card
  during the cooldown shows the daemon's own text (the card is opaque to
  the desktop, which does not know whose it is before the daemon reads it).
  The desktop never resends on its own. `Unavailable` (the home
  node unreachable, or the daemon busy) is `backend.contact.unavailable`:
  nothing was added, try again later.
- `contact.requests.list {}` → `{requests: [row], contact_requests: "manual" |
  "auto_accept"}`: the pending incoming requests and up to 128 of this
  account's own (pending first, then the newest accepted / declined /
  expired; closed rows are pruned after an hour). An incoming request past
  its expiry is left out at once, and an own one shows `expired` at once,
  whether or not the daemon's maintenance ran. A row: `request_id`,
  `link_id`, `principal_id`, `root_commitment`, `direction` (`inbound` /
  `outbound`), `state`, `expires_at` (Unix seconds), `note` and `preview`
  (delimited, sanitized quoted fields, product
  `text_safety::quote_fallback_field`: `"…"` with `\"`, `\\`, `\n`,
  `\u{…}` escapes; preview at most 256 characters of the first held
  message). The desktop shows both exactly as sent, untrusted text, never
  quoted again ("Note: "…""), `message_count` (held; "1 message" /
  "N messages"), `safety_number` (exactly twelve digit groups, joined by
  spaces for display; anything else is shown as no number to compare,
  "you can't verify who this is yet", never a shorter one). Locked accounts
  cannot list. Read with every account snapshot and polled until the daemon
  refuses it as a method it does not know (`ValidationFailed`, "local bus
  error: unsupported local bus method: …"): only then are the request
  methods declared unsupported (`backend.gap.contact_requests`). Any other
  refusal (a locked account, an outage, a row the daemon could not read,
  a timeout) is a failed read: the requests already known stay answerable,
  the Requests section says so (`backend.contact.requests_unreadable`), and
  the next poll or request event reads the list again and clears it.
- `contact.accept | contact.decline | contact.block {request_id}` →
  `{request_id, principal_id, direction, root_commitment, state}` (accept
  also `contact`); `contact.request.*` are aliases. Each selector (`request_id`,
  `link_id`, `root_commitment`, `principal_id`) matches only its own column;
  the desktop sends `request_id` only. The same decision again succeeds;
  another decision on a closed request is `ValidationFailed` /
  `request_closed`, an unknown id `request_not_found`
  (`backend.contact.request_closed` / `request_not_found`). While the
  daemon's contact request capacity is exhausted (`ContactRequestCapacity`,
  r3) it refuses with the bus's untyped `Unavailable`; for a decision or the
  setting the desktop says nothing changed and to try again later
  (`backend.contact.request_unavailable`), keeps the request, and does not
  retry on its own. Accept delivers the
  held messages as ordinary `message.received` / `message.list` with their
  original send times (the desktop reads the conversations again). Decline
  sends the requester a decline; block tells the requester nothing (their
  request looks pending until it expires). Block needs the hold, whose
  strip names the requester and the consequence (`confirm.contact.request_block`:
  their request and waiting messages are deleted, and they can't message or
  ask again until unblocked). Once a request is answered or expired the
  desktop drops its note and preview; the translator keeps only the id and
  state of this account's own requests (for the cooldown explanation).
- `contact.requests.policy.get {}` / `contact.requests.policy.set
  {contact_requests}` → `{contact_requests}`.
- Fences (r3/r4, daemon side; nothing for the desktop to send): a decline or
  block fences the peer's requests and pair controls; `contact.unblock`
  clears the fence; adding a peer whose request this account declined, or
  accepting one, clears this side's decline fence and admits the peer's
  answer; an accepted contact is never fenced.
- Events (stream topics and the push subscription): `contact.requested`
  (a new incoming request, and again as held messages arrive),
  `contact.accepted`, `contact.declined`, `contact.blocked`,
  `contact.request.expired`. A body carries identifiers and counts only:
  `request_id`, `principal_id`, `root_commitment`, `state`, `direction`,
  `message_count`; never the note, preview or safety number. Each is shown at
  once and calls for `contact.requests.list` (where the private fields are
  read while unlocked), `contact.list` and the identity directory; an accept
  also for the conversation directory. A requester's own pending request has
  no event; its decline arrives as `contact.declined` (`outbound`).

**Contacts dock.** Requests list the pending ones (who asks, the note, "N
messages waiting until you answer" with the preview, the safety number, Block /
Decline / Accept); the switch "Accept contact requests automatically" shows once
the daemon reported the setting, under the Requests heading (shown from then
on), with help for each state (On: new requests are accepted at once; those
already waiting still wait). A contact this account asked says, in quiet
text, "Waiting for X to accept your request" with Send again, "X declined your
request" (nothing to resend; asking again in the add form during the cooldown
is answered plainly), or "Your request to X expired" with Send again.
Screenshots: `request_shots` (run explicitly).

## Root-signed public identity profiles

`identity.directory` supplies `kind: human | headless_ai | bot` and
`kind_verified:true` when a public profile is authenticated by the same root that
signs the device set. This proves the root's claim, not biological humanity. It
is independent of manual contact `verified` and `safety_state`; do not show a
verified safety number solely because kind is authenticated. Legacy/profile-less
identities keep `kind_verified:false`. `kind_asserted` is a kind string or null,
never a boolean.

`contact.add` by `principal_id` returns the profile's `kind_asserted` and
`display_name`; a stale card assertion cannot override authenticated profile
fields. Public names are NFC single-line text, at most 128 scalars/256 UTF-8
bytes. Private account labels are not published. The default public name is the
principal ID when it fits this policy; otherwise the fixed safe name `Account`
keeps legacy long/unsafe identifiers publishable. Profile epochs cannot roll back
or equivocate across restarts.

Task actors include `actor_kind: human | headless_ai | bot | null` and
`actor_kind_verified`. Verification requires a root-signed profile and, for an
event, a current device of that principal. Actor rows also include `device_id`
when an event supplies it (empty when no event device is known). A human
relation role is not marked
kind-verified for a signed bot/headless-AI profile. Task relation authorization
itself is unchanged; do not turn this metadata flag into a permission grant.
Device-set v2 carries the profile; old v1-only implementations reject it, so
client/server upgrades must be coordinated. No new daemon method is introduced.

Public names are untrusted labels. The protocol rejects default-ignorables,
blank-rendering fillers, leading combining marks, names without a visible letter
or number, and verification-like checkmarks. Non-Latin and mixed-script names
remain supported. Directory/contact/request rows include `mixed_script: boolean`
and `display_name_confusable_with: string | null` (another known principal ID).
The latter compares warning-only UTS #39 skeletons against known cached names,
including the account's own name; it is not identity equality or an authorization
check. Clients must show a warning badge when either fact is present and put the
principal ID or root fingerprint next to a confusing name. A kind-authentication
chip belongs to the kind claim, never to the name; a malicious root can choose a
misleading name. Local private labels may take precedence in the UI.

`contact.add`, `contact.list`, and `contact.requests.list` expose `kind_verified`
alongside `kind_asserted` and `display_name`. A card-only label has
`kind_verified:false`. An authenticated rename emits `contact.changed` with
`reason:"public_profile_renamed"`, `principal_id`, `old_display_name`, and
`new_display_name`; refresh contact and identity rows and show the former name
when appropriate. A rename never refreshes manual safety verification.

`device.refresh.kind_asserted` is now a kind string or null and `kind_verified`
means a verified public profile, matching directory/contact rows.
`safety_kind_verified` retains the separate manual safety-and-authority check.
`identity.directory.safety_state:"verification_stale"` remains mandatory when the
manually verified full authority changes, even if the active-device hash and
profile are unchanged. All task outcome, pending, rejection, transport and
history metadata use the event's relation role when reporting
`actor_kind_verified`; a headless-AI profile cannot authenticate a human role.
A corrupt cached profile makes that actor unverified without failing unrelated
rows. These pre-1.0 field semantics require coordinated client/server releases.

## Full backup and restore (DESK-F22, on BACKUP-01)

Owner requirement: a full backup has no count limits and holds everything
(all messages, groups, contacts and every attachment); a restore verifies
the whole archive first and only then installs it in one step; restoring
onto an account already on this device **merges**, the newer item winning;
both show progress, and a restore ends with per-category counts and the
attachment hash check. The product implements the daemon job API below
(the product's `docs/account_backup.md` is normative; this section states
what the desktop does with it). Each account snapshot reads
`account.backup.jobs`: a daemon that answers it has the job API, the backup
methods are declared supported and the scope opens its job link (below);
until then (or with `SdkConfig::backup_api` false) they are declared
unsupported (`backend.gap.full_backup`) and Settings · Backup shows its
buttons disabled with that reason. The daemon snapshot reads it too: with
no account open (the entry page of a fresh device) the same job API runs a
full restore at daemon scope. The older `account.backup.export` /
`account.backup.import` (manifest and contacts only) stay as they are:
Settings · Account calls it a key backup, and the entry page offers it as
"Key backup" beside the full restore.

**Store.** A `BackupJob` (`BackupJobId`, `kind` Backup | Restore, `path`,
`phase`, the daemon's rising per-job `version`, `started_at`,
`finished_at`, `progress {category, items_done, items_total, bytes_done,
bytes_total}`, and once known: a backup's `summary {counts, file_bytes}`, a
verified restore's `preview {account_id, account_label, created_at, counts,
file_bytes, merge, attachments}`, a finished restore's `report {restored,
kept_newer, attachments}` (both counts per category), a failure) arrives
whole as `DomainEvent::BackupJobUpdated`. `BackupCounts`: contacts,
conversations, messages, groups, surfaces, tasks, attachments,
attachment_bytes. `AttachmentCheck`: verified, mismatched, missing,
unavailable_at_source (included in missing), local_missing_kept;
`all_verified` = no mismatch and every missing file recorded as unavailable
at the source. `Account.secure_sessions` (Ready | Reestablishing) says
whether the account's outbound secure sessions are usable (below,
"Outbound security after restore"). Phases:

```text
Backup:  Preparing -> Writing -> Done            (either: -> Failed | Cancelled)
Restore: Verifying -> Verified -> Installing -> Done
         Verifying | Verified -> Failed | Cancelled;  Installing -> Failed
```

A backup `Preparing` on `attachments` with an item total is fetching the
files not yet on this device (`BackupProgress::is_fetching`). The reducer
refuses a version that does not rise, a changed kind or path, an illegal
transition, and a first report in a phase the kind does not have.
`BackupFailure`: `WrongPassphrase`, `Tampered`, `Truncated`,
`UnsupportedVersion`, `InsufficientSpace {needed, available}`,
`Unreadable`, `NotWritable`, `StagingExpired`, `Interrupted`,
`AttachmentMismatch {checks}`, `AttachmentsUnavailable {count, selectors,
next_cursor}`, `IdentityRollback`, `RootConflict`, `Other(code)`.
Selectors: `running_backup_job`, `latest_backup_job` (of two jobs started
in the same second, the one seen last).

**Intents** (all human only; agents never back up or restore).
`CreateBackup {account_id, path, secret, allow_missing_attachments}` →
`backup.create` (`allow_missing_attachments` is false unless the user
explicitly asked to record unavailable files after a backup failed on
them), `VerifyRestore {path, secret}` → `restore.verify` (both
`SecretCall`; the passphrase never enters a body, an event, the store or
the inspector; one job at a time: `BackupInProgress`), `InstallRestore
{job_id}` → `restore.install` (only a `Verified` restore:
`BackupJobPhase`), `CancelBackupJob {job_id}` → `backup.cancel` (until a
restore installs). The SDK plans `backup.create` for the account the intent
names, which must be the scope's own; a backup of another account is
refused.

**Daemon job API.** Jobs belong to the daemon, not to an account; one runs
at a time. Create is sent in the scope of the account backed up; verify,
install and cancel carry no account (a restore may be of an account not on
this device). Job changes arrive only on the daemon-level job stream: a
`bus.subscribe` with no `account_id`, topics exactly
`["account.backup.job.changed"]` and `attended_frontend: true` (any other
topic there is refused). The daemon takes the four job requests only from a
connection that opened that stream as an attended frontend, so the SDK
keeps one such connection per scope (the job link,
`ramflux_desktop_sdk::bus::jobs`): opened once a snapshot found the job
API, and carrying every job request. The daemon scope keeps its link (and
its job polls) only while no account scope is attached; an account's own
link follows the jobs then, so there is one attended connection and one
set of job reads at a time. The event frame's `seq` is the
daemon's job stream cursor; the row's `seq` is the job's version. The subscribe answer's
`head_seq` seeds the link's cursor, so a job whose events the link never
saw (started elsewhere, or while the link was down) is covered too. The link
reopens from the last cursor (`since_seq`); a cursor the daemon no longer
covers answers `resync_required` and the link follows from the head;
`stream.lagged` or a lost connection ends it and it is opened again (at
once, or after the reconnect back-off). Each time it opens,
`account.backup.jobs` is read again. `account.backup.jobs` is also read
with every account snapshot and with every poll while a job runs (it works
in the account's scope and with none).

| Request | Body | Answer |
|---|---|---|
| `account.backup.create` | account scope; `output_path`, secret `passphrase`, `allow_missing_attachments` (default false) | `{job}` (`preparing`) |
| `account.restore.verify` | `input_path`, secret `passphrase` | `{job}` (`verifying`) |
| `account.restore.install` | `job_id` | `{job}` (`installing`) |
| `account.backup.cancel` | `job_id` | `{job}` (`cancelled`, once its worker stopped) |
| `account.backup.jobs` | `{}` | `{jobs: [job]}` running and recent, newest first |
| `account.backup.jobs` | `job_id`, `missing_after` | `{job, unavailable_attachments, unavailable_count, unavailable_next_cursor}`: a page (64) of a failed backup's unavailable files |

Event `account.backup.job.changed`, body the job row. Job row: `job_id`,
`kind` (`backup`/`restore`), `state` (`preparing`, `writing`, `verifying`,
`verified`, `installing`, `done`, `failed`, `cancelled`), `seq`, `path`,
`created_at`, `finished_at` (Unix seconds), `progress {category,
items_done, items_total, bytes_done, bytes_total}` (always present;
`category` one of `identity`, `contacts`, `conversations`, `messages`,
`groups`, `surfaces`, `tasks`, `settings`, `attachments`; a row with another
category is dropped); `counts {contacts, conversations, messages, groups,
surfaces, tasks, attachments, attachment_bytes}` (a backup's snapshot once
it writes, a restore's authenticated archive) and `archive_bytes` (a
finished backup, a verified restore); `archive {local_account_id,
account_label, created_at, merge}` (a verified restore; `merge` = the
account is already on this device); `restored` and `kept_newer`
(per-category count objects with the fields of `counts`, a finished
restore); `attachments {verified, mismatched, missing,
unavailable_at_source, local_missing_kept}` (a verified, failed or finished
restore); `error {code, needed_bytes, available_bytes, unavailable_count,
unavailable_attachments, unavailable_next_cursor}` (a failed job). Error
codes: `wrong_passphrase` (the job's first failure, right after header
authentication), `tampered`, `truncated`, `unsupported_version`,
`insufficient_space` (checked before and while writing; with the figures),
`unreadable`, `not_writable`, `staging_expired`, `interrupted`,
`attachment_mismatch`, `attachments_unavailable`, `identity_rollback`,
`root_conflict`. A refused job request is `BackupRefused` with the reason as
`reason_code`: a job error code, or `busy` (a job runs), `account_locked`,
`unknown_job`, `invalid_job_phase`; each has its locale key under
`backend.backup`. A restore that reaches `done` makes the source read the
account again, and the account stream's `account.restored` (after the usual
change events) reloads the account it names.

**Verification and staging.** Verify authenticates the header, reads the
whole archive into encrypted staging, checks every file against its hashes
and only then reports `verified` with the preview. Every file the archive
holds must match and every missing file must be one the backup recorded as
unavailable at its source; anything else fails the job
(`attachment_mismatch`, with the check counts) and nothing is installed. An
archive whose identity root differs from the account here is refused
(`root_conflict`); one that would roll a *new* install's identity back to
an older epoch is refused (`identity_rollback`; a merge keeps the live
identity and accepts older backups). Verified staging is kept, encrypted
under a RAM-only key, until install or 30 minutes, whichever comes first;
cancel, failure, expiry (`staging_expired`) and a daemon restart discard
it; the passphrase is never kept. Install stops only the account restored,
merges or adds it, and reopens it; cancel is refused once installing.

**Merge.** Installing onto an account already on this device merges: where
both have an item the newer one is kept, including newer removals, blocks
and other restrictions (a deletion wins a tie); `kept_newer` counts, per
category, what this device had in a newer version. Files this device
referenced but had not downloaded yet stay as they were
(`local_missing_kept`), still fetchable; they are not archive omissions.

**Unavailable source files.** A backup first fetches every file the account
references but does not hold (the `preparing` phase reports the files tried
of the estimated total), then takes its snapshot. Files that stay
unavailable fail it, typed `attachments_unavailable` with
`unavailable_count`, the first page (64) of their opaque selectors and
`unavailable_next_cursor`; the remaining pages are read with
`account.backup.jobs {job_id, missing_after}` until the cursor is null (the
diagnostics live 30 minutes). Nothing is written. The user may make the
backup again with `allow_missing_attachments: true`, explicitly: it still
tries to fetch first, then records the remaining files (with their expected
hashes) in the archive's authenticated omission manifest; verify and
install report them as `unavailable_at_source`. The default stays strict.

**Daemon restart.** Jobs live in the daemon's memory. When the job link
reopens and its cursor is no longer covered (`resync_required`), the SDK
reads the jobs again and reports every job it knew as running that the
daemon no longer lists as `Failed`, `Interrupted`, so the next job may
start. This covers a job known from the listing only, since the cursor is
seeded from `head_seq`. Only a method the daemon does not know (or a
`CapabilityDenied` for a non-human account) marks the job API missing; any
other refusal of the `account.backup.jobs` read is a read failure.

**UI (Settings · Backup).** Full backup: Create backup… opens the form:
location (native save dialog; a file already there is refused in the form,
since the daemon never replaces a file), passphrase typed twice (at least
12 characters and at least 16 UTF-8 bytes, one rule in
`settings::backup::text::Strength`; a strength hint; a mismatch is named),
Create backup. While it runs: what it is working on ("Fetching
files not yet on this device: n of m" while fetching, else the category),
a progress bar (bytes, else items), bytes and items, the file, Cancel
backup. Then: file, size and every count, Done. A backup that failed on
files it could not fetch says how many and offers, explicitly, Back up
without them…: it reopens the backup form to the same file with a note
naming how many files it records as missing; the passphrase is typed again;
the next backup is strict again. Restore: Restore from backup… opens the
form: backup file (native open dialog), passphrase, Check backup. While
checking: progress and Cancel restore. Checked: "Restore this backup?" with
the account, when it was made, its size, counts and file check (every file
matched; files recorded as unavailable when it was made are named), and
what installing does (merge, newer wins, including newer removals and
blocks; or the account is added), Cancel (discards the staged copy) and
Restore. Installing: progress, no cancel ("the data on this device stays as
it is until the restore finishes"). Done: the file check, the files
recorded as unavailable at the source and the files this device had not
downloaded yet, the restored counts, and what was kept because this
device's was newer, per category (or that nothing here was newer).
Failures are shown typed, with Try again… (the file is kept); a file that
does not match fails the check and nothing can be restored. A refused
request (for example `busy`) is explained in the form. Only one job runs at
a time: while one runs it replaces both forms.

**UI (entry page, fresh device).** Restore from backup offers "Full backup"
and "Key backup"; the full backup is the default when the daemon has the
job API, else the key backup. The full restore is the same panel at daemon
scope: file, passphrase, Check backup, the review, Restore, the result; it
needs no account. The daemon-level job stream and `account.backup.jobs` are
read in the daemon scope too, so the job is followed exactly as in
Settings; the entry page's panel follows only the jobs it started. When
the restore is done the SDK reads `account.list` again. The result
(per-category counts, what was kept as newer, the file check) stays on the
entry page until the user closes it: with the unlock form when the daemon
lists the restored account locked, and with "Open account" when the daemon
reopened it unlocked already (the shell holds the entry page, instead of
switching to the workspace, while that result is in view).

**Outbound security after restore.** Full restore keeps all historical
messages, attachment keys and inbound/archived ratchets. Saved outbound DM
sessions and group sender chains are archived only; they are never
resumed. A newly restored account performs fresh root-authorized device
activation, starts fresh outbound DM sessions on the first post-restore
message and rotates every group's sender key to a new chain ID before its
first group send (v3 sender distributions: every group participant must
upgrade together). Merge retains the live local outbound state and does not
reactivate backup outbound chains. Meanwhile `account.status` and
`sync.status.get` report `secure_sessions: "reestablishing"`, sending is
refused by the daemon, and `account.secure_sessions.changed` announces
`ready`; history and files stay readable. The desktop keeps it on
`Account.secure_sessions`: read with every `account.status`, set by
`account.restored` (which carries it) and by
`account.secure_sessions.changed` on the account stream. While it is
`Reestablishing` the conversation workspace shows a persistent banner
("Re-establishing secure sessions…") until the daemon says `ready`; a send
that fails meanwhile shows the daemon's reason in the status strip.

## Verified peer kinds (DESK-F23, on KIND-01)

How the desktop reads the section above, aligned with product main
`eae12fb` (KIND-01 merged: `docs/identity_profiles.md`,
`client/identity_authority/profile_read.rs`, `bus/read_model/rows.rs`,
`client/contact_resolution.rs`, `client/task/read.rs`). The fake daemon
(`ramflux-desktop-test` `fake_profiles`) joins the same fields.

**Kind claim.** `kind_asserted` (contact, request and `contact.add` rows)
and `kind` (identity rows) name `human`, `headless_ai` or `bot`, or null;
`kind_verified` says a root-signed profile states it. The desktop keeps one
`KindClaim`: `Verified(kind)` when the flag is true, `Asserted(kind)` when
a kind is named without it (a card, or a legacy device-set kind), `None`
when null, whatever the flag says. A verified claim from either list wins;
otherwise the identity row's, then the contact row's. `ParticipantKind`
stays the coarse classification (`Human`; `Agent` for both AI kinds;
`Unknown` for `None`). A workflow principal names no peer kind. A claim is
never a verified safety number: safety stays what `safety_state` says.

**Safe default.** A principal nothing was reported about (a name lookup, a
legacy identity without a profile, a row from an older daemon) is
`KindClaim::None`, `SafetyState::Unverified`, no name warning, shown by
its short id: unverified, its kind unknown.

**Stale verification.** `safety_state: verification_stale` is
`SafetyState::VerificationStale`: it `needs_attention` (sticky banner in
warning tone with Verify again) but refuses nothing. A new conversation, a
new group and a call show an inline acknowledgement (Verify again /
Continue) and then go ahead; only a key change or a fork refuses them
(`SafetyState::refuses_contact`). The wording never claims the keys changed:
the product sets it when the manually verified device-set hash changed, when
a signed authority change (such as a revoked device) alters the verified
authority, and on a representation upgrade (`rows.rs`); the desktop needs no
second signal.
`safety_kind_verified` is reported only by `device.refresh`, which the
desktop does not call for peers; the identity row's `safety_state` is the
manual verification the desktop shows. (The section above lists it with
the directory rows; the product exposes it on `device.refresh` alone.)

**Name warnings.** `mixed_script` and `display_name_confusable_with`
(contact, identity and request rows) are `NameWarnings`. Either shows a
warning with the full principal id (never shortened; tooltip and copy)
next to the name — on the contact,
the request, the member, the direct conversation's header and the author
of a message. It never blocks anything and never marks the name verified;
the kind mark belongs to the kind claim, never to the name.

**Rename.** `contact.changed` with `reason: public_profile_renamed`
(`principal_id`, `old_display_name`, `new_display_name`) becomes a
`MomentKind::NameChange` in every known context the principal is in, by
the principal, shown with the full principal id, keyed and timed by its
stream entry and remembered across restarts; the contact and identity rows
are read again (the stream already does). A replayed announcement adds
nothing. A rename never touches the safety state.

**Task actors.** `actors.{requester,assignee,acceptor}` (`principal_id`,
`device_id` — empty when no event device is known — `actor_kind`,
`actor_kind_verified`) are the task's `TaskActors`; the card shows the
assignee's kind as verified for the task, and the requester's on "Asked
by". A view without `actors` names the assignee alone with the view's
`actor_kind_verified`. A task message row (`content_type: task`) carries
`actor_kind_verified`; its line shows the signer's kind when verified,
"kind not verified" otherwise. A sibling copy without the field is read
from its text (an accept, a decline and a decision name an unverified
actor; a cancel never says, so it is unverified). None of this is a
permission.

Screenshots: `peer_kind_shots` (run explicitly).

## Account creation with a recovery phrase (DESK-V3-01, on ID-02-IMPL-1)

Built contract-first on the mock source against the five ceremony methods
(begin, show-once, hidden-challenge, confirm, cancel; the product's
`docs/root_key_custody.md` is normative for the ceremony rules), and wired to
the real SDK source in DESK-V3-02 (see "Wiring to the SDK source" below, from
the product's `docs/local_bus_account_creation.md`). The sections up to there
describe the screens and the store, which both sources share.

**Capability.** `Capabilities.born_v3_creation` is false unless the daemon
reports `CapabilitiesUpdated` with it true. `unsupported` has no say in it:
a daemon that lists nothing unsupported still does not offer the ceremony.
The entry page's account step shows "Create account with recovery phrase"
in place of the passphrase form while it is true: a daemon that offers the
phrase has no other way to create an account (final contract, ID-02-IMPL-3
decision r1: `account.create` takes no seeds and starts the ceremony; the
steps run on the same local-bus connection; cancel or a lost connection
erases it; unlock still uses the account passphrase). An older daemon keeps
the passphrase form. Policy refuses `BeginCeremony` without it
(`CeremonyNotOffered`).

**Store.** A `Ceremony` (`phase`, `ceremony_id`, `label`, `expires_at`) arrives
whole as `DomainEvent::CeremonyUpdated`. Phases:

```text
(none | ended) -> Prepared -> PhraseReady -> Challenged{positions} -> Created(account)
Prepared | PhraseReady | Challenged -> Failed(code) | Cancelled
(none | ended) -> Failed(code)             a refused begin: empty ceremony_id
```

The reducer refuses a phase the ceremony may not move to
(`CeremonyPhase::can_follow`), a changed ceremony_id or expiry on a running
ceremony, and a late failure of an ended one (it would overwrite the
record). `CeremonyFailure`: `Disabled`, `ConfirmationFailed` (the only
attempt is spent), `VaultUnavailable`, `Expired`, `Invalid`. The
`expires_at` is five minutes (`CEREMONY_TTL_MS`) after begin.
`Store::ceremony_revision` counts the updates, so a view that opens over an
ended record can tell the next ceremony's news from the old one.

**The phrase is not in the store.** It is a secret on its way to the
screen, like a call's media session: `Source::take_recovery_phrase` hands
out the `RecoveryPhrase` once, after the ceremony reports `PhraseReady`. It
is neither `Clone` nor `Serialize`, its `Debug` is `<redacted>`, and its
buffer is zeroized on drop. No event, scenario, store row, effect body or
inspector entry carries a word; the words the user types back are
`SecretInput`s beside an empty body, as the backup passphrase is.

**Intents** (all human only). `BeginCeremony {label}` → `account.create`
`{display_name, principal_kind: "human"}` (a person's account only: an AI identity needs a custodian the desktop does not offer) (no seeds, no secret; answers
`{ceremony: pending, ceremony_id}`); `ShowRecoveryPhrase` →
`account.create.show_once`; `HideRecoveryPhrase` →
`account.create.hidden_challenge`; `ConfirmCeremony {words, passphrase}` (exactly three
non-empty words in the order asked, then the account passphrase: four secrets) → `account.create.confirm {words, passphrase}` (answers the
ordinary account-create response: account id, unlocked, networking `live` or
`networking_pending`); `CancelCeremony` → `account.create.cancel`. Each is allowed only in its
phase (begin when none runs; show in `Prepared`; hide in `PhraseReady`;
confirm in `Challenged`; cancel while it runs).

**Screens** (`entry/ceremony`):

1. Name. Continue begins; Back leaves.
2. Get ready: write down offline, no screen sharing or recording (the app
   cannot block it), the words appear once; Show recovery phrase.
3. The 24 words, numbered, in a grid; the warning (write down offline,
   never share, anyone with them owns the account); no copy button. The
   words are **covered** (not drawn) from the moment the window loses focus
   and come back only on request while it is active. "I wrote them down"
   erases the phrase from the UI state *first* and only then sends
   `HideRecoveryPhrase`, so the daemon chooses the positions after the
   screen is empty.
4. Type three words back (Continue keeps them as secrets in the panel and sends nothing), then set the account passphrase: typed twice under the rules of the backup passphrase (`settings::backup::text::Strength`: at least 12 characters and 16 bytes, strength hint, a mismatch named) with the same copy; one submit sends `confirm {words, passphrase}`. The passphrase unlocks the account on this device like any other. Words and passphrase are held only as `SecretInput`s and dropped on cancel, expiry, failure and close.
   The words are asked at the positions the store reports. Each input
   completes from the English BIP39 list (`recovery::words`): the word or its
   first four letters (unique in the list), read as the backend reads it
   (NFKD, ASCII lowercase, one token, at most 128 bytes) and sent as the full
   list word. A word that is not in the list is shown as such and the
   button stays off, so the single attempt is not spent on a typo. The
   typed text is cleared at once. A wrong word ends the setup
   (`Failed(ConfirmationFailed)`): no second attempt, the error is shown.
5. Done: the account exists and is unlocked. When its networking is
   `pending` the screen says so ("Networking pending": not registered with
   any node, cannot send or receive until that step exists); when `live` it
   says connected. The ceremony adds no account to the store itself (the
   account arrives through the ordinary account events), so the shell stays
   on the entry page until it does.

The time left is on every step from 2 to 4. When the five minutes pass the
panel drops the phrase and the typed text, sends `CancelCeremony` so the
daemon lets go, and keeps "expired" in view (not "cancelled"). Cancel is on
every step.

**Limits.** GPUI Kit 0.8.0 offers no way to exclude a window from
screenshots or screen sharing, so the screen asks and cannot enforce. The
words drawn for a frame are copied into GPUI text elements, and GPUI's text
layout caches may keep shaped glyphs for a few frames: those copies are
outside the panel's control and are not zeroized; they stop being refreshed
the frame the words stop being drawn. The typed words live in GPUI Kit
input buffers until cleared (`set_value` also clears the undo history).

**Mock** (`ramflux-desktop-test::mock_ceremony`, the built-in answer to the
five methods; a scenario rule for one of them wins). The phrase is a fixed
non-production sequence of 24 different list words, the positions asked are
`DEMO_POSITIONS` unless a test sets them, a step out of order or after the
deadline ends the ceremony (`Invalid`, `Expired`) and erases the phrase,
confirm consumes the ceremony whatever the outcome. Fixture `s13_born_v3`
reports the capability after two seconds.

**Pending requests (r2).** Every control that sends a request (continue,
show, "I wrote them down", confirm, cancel) is off and busy from its press
until the daemon answers (a newer ceremony revision, or a newer failed
effect, or a refused intent); the methods refuse a second send as well. While
the hide is awaited the screen shows a neutral "choosing three words" line
(status, not alert); "no longer available" is for a phrase that was never
received after the daemon answered.

**Networking state on the account (r2).** `Account.networking` (`Live`,
`Pending`; default `Live`) is the account list's state. The mock adds the
account (unlocked, `Pending`) with the ceremony's result. The done screen
stays in view until closed even though the account is listed, then the
workspace shows a persistent "networking pending" banner and the account
rows of the entry page a tag, for as long as the list says `Pending`.

**Keyboard and window (r2, r3).** Focus moves when a screen first appears,
and never to a button that cannot be undone: to the input to type in; to the
word list when the 24 words appear (a labelled list read in order; "I wrote
them down" is reached by Tab only, so a stray Enter or Space hides
nothing); to the pending line while three words are chosen; otherwise (DESK-V3-02)
to the screen's heading, or to its error when the screen is only an error. Escape asks "Cancel this setup?", moves focus to the question
and gives it back to where it was on "Keep going" (Escape again dismisses;
the Cancel buttons cancel at once). Back or Escape while the begin request is
in flight remembers the abandoned request: the ceremony it opens is cancelled
when the answer arrives, and a new setup waits for that (its Continue is busy).
Closing the window sends `account.create.cancel` (best effort; the daemon
also lets go when the connection drops). Word cells are at least 8.5rem wide
and wrap to fewer columns instead of clipping; the application window has a
minimum size (`MIN_WINDOW_SIZE`, 640 x 480) and the narrow-width test opens at
it. A pending request ends on its own answer or the failure of its own
method, not on another effect failing.

**More limits (r2).** The grid's per-frame copies of each word (the cell
text and its accessibility label, about two per word per frame while drawn;
the countdown redraws every second) are plain strings in the freed heap and
are not zeroized. The cell label "Word N: <word>" is published to the platform
accessibility tree, which any process with accessibility permission can read,
not only a screen reader. Both match what is on screen; they are accepted and
documented, not removed.

**Passphrase copies (r3).** The typed passphrase and its repeat live in GPUI
Kit input buffers. The panel reads each once per frame of the passphrase
step (`InputState::value()` materializes a new string; the panel borrows it
for the strength rating, the mismatch and the button state, no further
copies). The one send reads each input once more (two `SharedString`s, used
for the rating, the comparison and the secret) and moves one `String` into the
`SecretInput`: three transient copies, of which only the last is zeroized with
the secret. Those frame copies, the
input's own rope and undo-free buffer and the masked text layout are plain
strings outside the panel's control and are not zeroized; the inputs are
cleared with `set_value("")` as soon as the secret is sent, cancelled or
expired.

## Wiring to the SDK source (DESK-V3-02 r1, on ID-02-IMPL-3)

Product source of truth: `docs/local_bus_account_creation.md` of the product
revision the workspace is pinned to (`Cargo.toml`; on this branch the
ID-02-IMPL-3 r5 checkpoint, to be replaced by the merged revision before
this merges). The nine additions the DESK-V3-01 contract asked for, and what
the desktop does with each:

1. **Capability read.** `daemon.status.born_v3_creation`; absent means false.
   `ramflux_desktop_sdk::capabilities::Found::born_v3` carries it into
   `Capabilities.born_v3_creation` on every status poll (it can be withdrawn
   too). `unsupported` still has no say in it.
2. **Expiry on the begin answer.** `expires_in_ms` (remaining time, at most
   five minutes) becomes `Ceremony.expires_at` (the bridge's clock when the
   answer arrived plus the remaining time).
3. **Begin input.** Exactly `{display_name, principal_kind: "human"}`. The
   daemon allocates every identifier. Desktop v1 offers `human` only.
4. **Typed confirm answer.** `{account_id, display_name, state: unlocked,
   networking: live | networking_pending}` becomes the ceremony's
   `CeremonyAccount` and, before it, an `AccountUpdated` through the
   translator (so the account list the translator keeps agrees).
5. **Expiry code.** `CustodyExpired` is `CeremonyFailure::Expired` (the screen
   states expiry when its own clock and the daemon disagree);
   `CustodyConfirmationFailed` is `ConfirmationFailed`,
   `CustodyVaultUnavailable` is `VaultUnavailable`, `CustodySetupInvalid` is
   `Invalid`. A connection that broke mid-ceremony is `Invalid` plus a failed
   effect (`Transport`); a begin that could not even connect is a failed
   effect only.
6. **Pending account visibility.** `account.list.account_details[].networking`
   and the unlock answer's `networking` fill `Account.networking`, so an
   account left `networking_pending` is listed, tagged and unlockable after a
   restart.
7. **Dedicated connection.** `bridge::custody` keeps one actor per scope that
   owns the only `LocalBusCreationClient` (or `LocalBusRecoveryClient`) of a
   ceremony, apart from the multiplexed request connection; a new begin, the
   end of the scope, a timeout or a broken connection drops it, which cancels
   the ceremony. The phrase travels as a moved `Zeroizing<String>` from the
   bridge thread to the GPUI thread through a plain channel (`Handoff`), is
   parsed in place by `RecoveryPhrase::parse` (no copy) and is erased by the
   source if it is not taken, belongs to an earlier ceremony, or the ceremony
   moves past `PhraseReady`. The event that says it is ready is sent after the
   move. No JSON value, event, log or request body ever holds it.
8. **Cancel after consumed.** The desktop does not send it then (the
   connection is dropped when confirm answered); a lost reply cannot wedge a
   retry because a new begin opens a new connection.
9. **AI identities.** Not offered: no kind choice, `human` only; a
   `headless_ai` begin would be refused as `CustodySetupInvalid` anyway.

**Test hook.** `SdkHub::phrase_audit()` counts the phrases the daemon handed
out (`issued`) and how each ended (`taken` by the screen, or `wiped` by the
bridge and checked empty afterwards), and the zeroizing copies made of typed
words and passphrases (`secrets` against `secrets_wiped`). The tests assert
that nothing is left outstanding after a taken phrase, an untaken one, a
cancel, a dropped source and a failed confirm.

**Where the ceremonies live in a live window.** A ceremony belongs to the
daemon's session. Once it opens the account (an unlocked account gets a
session of its own and is rendered), that session's store knows nothing of
the ceremony that opened it. So the entry panels read
`StoreHandle::entry_store()` (the daemon's store on a live daemon, else the
rendered one), the ceremony and recovery intents are decided against and
submitted to the daemon session whichever account is rendered, the phrase is
taken from the daemon session, and the entry page shows the held result (the
done screen) before its "waiting for the account's session" line.

### Recovery from the phrase

`account.recover.begin {account_id}` returns `{ceremony_id, account_anchor,
expires_in_ms, warning}`; `account.recover.confirm {phrase, passphrase,
confirmed_anchor}` returns the created-account answer; `account.recover.cancel`.
The passphrase is the account's own passphrase on this device (recovery does
not change it); the phrase is checked against the account's master key; the
daemon says recovery revokes the account's other devices.

**Store.** A `RecoveryCeremony` (`phase`, `account_id`, `anchor`,
`ceremony_id`, `expires_at`) arrives whole as `DomainEvent::RecoveryUpdated`:

```text
(none | ended) -> Prepared -> Recovered(account)
Prepared -> Failed(code) | Cancelled
(none | ended) -> Failed(code)             a refused begin: empty ceremony_id
```

Same rules as the creation record (a phase it may move to, a fixed id, account
and expiry, no late answer to an ended one); `CeremonyFailure` is reused
(`ConfirmationFailed` for words or a fingerprint that do not match,
`Invalid` for a refused passphrase or a step out of order).
`Store::recovery_revision` counts the updates.

**Intents** (human only). `BeginRecovery {account_id}` (the daemon offers the
phrase, the account is a person's, no recovery runs) sends
`account.recover.begin`; `ConfirmRecovery {anchor, phrase, passphrase}` (the
anchor must be the one the daemon named, the phrase 24 recovery words, the
passphrase not empty) sends `account.recover.confirm` with the phrase and the
passphrase as secrets beside `{account_id, confirmed_anchor}`;
`CancelRecovery` sends `account.recover.cancel`.

**Screens** (`entry/recovery`), opened only from the user's own action,
"Recover with recovery phrase…" on a locked person's account of a daemon that
offers it:

1. Intro: one warning (recover only if you started this, every other device
   is removed, never type the words because something asked); Continue sends
   the begin (no secret).
2. Check: the account fingerprint in groups of four; the user types it (the
   comparison ignores the spaces) before the button can be pressed; the 24
   words in one masked input, counted and with the first word that cannot be
   one named; the account passphrase in a masked input; the time left.
3. Checking, then Done (the account is unlocked; networking state as for a
   created account) or Ended (mismatch, invalid, vault, expired, cancelled).

Nothing a network event carries can open it: the panel starts no request by
itself, the record it shows must be newer than the panel's opening and name
its account, and a record nobody asked for is ignored (the one exception is a
recovery the panel itself started and could not cancel). The phrase and the
passphrase leave their inputs as `SecretInput`s at the one send and the inputs
are cleared at once; cancel, expiry, failure, close and the window closing
clear them too. Escape cancels (nothing is ever displayed that needs
protecting). One window-close handler serves both panels.

**Focus and names (DESK-V3-02).** Initial focus never lands on a button: the
intro and the done screens focus their heading, the check screen the
fingerprint input, the checking screen its status line, an ended recovery its
error. The same holds for the creation ceremony: the ready and done screens
focus the heading, an ended ceremony and the "phrase is gone" error their
alert. Every focus target is a named node (heading, list, status or alert)
and draws the theme's focus ring while focused (`focus_ring_style`).

**Abandoned begin (DESK-V3-02, from review r3 L-1).** A begin the user backed
out of while it was in flight is tracked apart from the panel's pending
request (`entry::abandon::Abandon`). It ends whichever way the answer
arrives: a ceremony (cancelled once, then the cancel's answer), a refusal that
is a ceremony update, a failure of the begin itself with no update at all, or a
failure of the cancel (the ceremony that still runs is then shown with its own
Cancel). A reopen of the panel in between changes none of that.
