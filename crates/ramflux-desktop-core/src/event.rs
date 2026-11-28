// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Domain events: facts flowing from a source into the store.

use serde::{Deserialize, Serialize};

use crate::ids::{AccountId, ContextId, GrantId, MomentId, NodeId, StepId, TaskId, Timestamp};
use crate::model::{
    Account, AgentApproval, AgentCatalog, AgentCreation, AgentHostStatus, AgentJob, AgentRecord,
    AgentRosterState, AgentSettings, AppLock, AuditEntry, BackupJob, Call, Capabilities, Ceremony,
    ContactCard, ContactRequest, Context, DaemonInfo, Device, Disappearing, EffectFailure, Grant,
    GrantFailure, GrantState, Integrations, Lifecycle, Listing, LockState, Moment, MomentKind,
    NeedsMe, Node, NodeHealth, NodeProbe, Object, Participant, RecoveryCeremony, RequestFloors,
    RequestPolicy, StandingApproval, StepState, SyncStatus, Task, TaskAnswer, TaskState,
};

use crate::card_event::CardEvent;
use crate::model::mls_group::{
    DeleteResult, GroupEventKind, HistoryDirection, Invite, JoinOutcome, JoinRecord, LeaveResult,
    MemberRow, MlsGroupSummary, Post, SendState,
};
pub use crate::surface_event::{SurfaceEvent, SurfaceSnapshot};
use serde_json::Value as JsonValue;

/// Schema tag carried by every envelope. The store rejects any other value.
pub const EVENT_SCHEMA: &str = "ramflux.desktop.event.v1";

/// A sequenced, timestamped domain event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventEnvelope {
    /// Must equal [`EVENT_SCHEMA`].
    pub schema: String,
    /// Monotonic sequence assigned by the source; starts at 1.
    pub seq: u64,
    /// When the source observed the fact.
    pub at: Timestamp,
    /// The fact.
    pub event: DomainEvent,
}

impl EventEnvelope {
    /// Builds an envelope with the current schema.
    pub fn new(seq: u64, at: Timestamp, event: DomainEvent) -> Self {
        Self { schema: EVENT_SCHEMA.to_owned(), seq, at, event }
    }
}

/// Connection state of the source as a whole.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceStatus {
    /// Live.
    Connected,
    /// Not reachable.
    #[default]
    Disconnected,
    /// Reachable but the account is locked.
    Locked,
}

/// Every fact the store understands.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DomainEvent {
    /// Account inserted or replaced.
    AccountUpdated(Account),
    /// Node inserted or replaced.
    NodeUpserted(Node),
    /// Health of a known node changed.
    NodeHealthChanged {
        /// Node.
        node_id: NodeId,
        /// New health.
        health: NodeHealth,
    },
    /// Participant inserted or replaced.
    ParticipantUpserted(Participant),
    /// A contact's identity changed (`contact.identity_changed`): the change
    /// waits for the user, who accepts it in a sheet. A newer change replaces
    /// an older one.
    IdentityChanged {
        /// The contact.
        participant: crate::ParticipantId,
        /// The contact's @username, when the event named it.
        #[serde(default)]
        username: Option<String>,
        /// The anchor the contact had.
        previous_anchor: String,
        /// The anchor the contact has now.
        new_anchor: String,
        /// Whether the SDK flags the contact as an AI.
        #[serde(default)]
        ai_flag: bool,
    },
    /// A status read of a contact's identity (`contact.verification.status`).
    IdentityStateRead {
        /// The contact.
        participant: crate::ParticipantId,
        /// The state.
        state: crate::IdentityState,
    },
    /// The user's acceptance of a contact's new identity took effect.
    IdentityAccepted {
        /// The contact.
        participant: crate::ParticipantId,
        /// The anchor that was accepted. The pending change is cleared only
        /// when it is still this one; a newer change stays to be reviewed.
        /// `None` accepts whatever is pending.
        #[serde(default)]
        new_anchor: Option<String>,
    },
    /// The user's acceptance of a contact's new identity did not take effect.
    IdentityAcceptFailed {
        /// The contact.
        participant: crate::ParticipantId,
        /// Why.
        reason: crate::AcceptFailure,
        /// The anchor the refused acceptance named. A newer pending change
        /// than this one is never cleared by the refusal; `None` when the
        /// attempt is not known.
        #[serde(default)]
        attempted_anchor: Option<String>,
    },
    /// What `conversation.security` says about a direct conversation.
    ConversationSecurityRead {
        /// The conversation.
        context: ContextId,
        /// Its security.
        security: crate::ConversationSecurity,
    },
    /// A send was refused because the contact's identity change is not
    /// accepted (`identity_change_pending`). The message stays as a failed
    /// moment; nothing is sent again by itself.
    SendBlockedByIdentity {
        /// The conversation.
        context: ContextId,
        /// The contact.
        participant: crate::ParticipantId,
        /// The failed message, when the source knows its moment.
        #[serde(default)]
        moment: Option<MomentId>,
    },
    /// Context inserted or replaced.
    ContextUpserted(Context),
    /// Context and its moments removed.
    ContextRemoved {
        /// Context.
        context_id: ContextId,
    },
    /// New moment.
    MomentAdded(Box<Moment>),
    /// A moment is erased from this device (a delete for this device only):
    /// it leaves the store and every index. An unknown id changes nothing.
    MomentRemoved {
        /// Moment.
        moment_id: MomentId,
    },
    /// Lifecycle and/or payload of a moment changed. A new payload must be the same kind.
    MomentUpdated {
        /// Moment.
        moment_id: MomentId,
        /// New lifecycle; equal to the current one means unchanged.
        #[serde(default)]
        lifecycle: Option<Lifecycle>,
        /// New payload.
        #[serde(default)]
        kind: Option<MomentKind>,
    },
    /// An agent surface fact (its snapshot and the authority's answers).
    Surface(SurfaceEvent),
    /// A card runtime fact about an agent surface's card.
    Card(CardEvent),
    /// Object inserted or replaced.
    ObjectUpserted(Object),
    /// New pending grant.
    GrantRequested(Grant),
    /// Pending grant decided.
    GrantResolved {
        /// Grant.
        grant_id: GrantId,
        /// Final state; must not be `Pending`.
        state: GrantState,
    },
    /// The assignee's full task update. The version must rise; state and
    /// step transitions, and where a step needs the requester, are
    /// validated.
    TaskUpserted(Task),
    /// The assignee's update of one step. The version must rise; the task
    /// state follows (a step that needs the requester makes a running task
    /// need them, and the last such step answered makes it run again).
    TaskStepChanged {
        /// Task.
        task_id: TaskId,
        /// The task's next version.
        version: u64,
        /// Step.
        step_id: StepId,
        /// New step state.
        state: StepState,
        /// Where the step asks the requester; required when `state` is
        /// `NeedsMe`, otherwise the step keeps its record.
        #[serde(default)]
        needs_me: Option<NeedsMe>,
    },
    /// The assignee's update of the task state alone (for example the
    /// answer to a cancel). The version must rise. When the task ends, its
    /// unfinished steps are skipped.
    TaskStateChanged {
        /// Task.
        task_id: TaskId,
        /// The task's next version.
        version: u64,
        /// New state.
        state: TaskState,
        /// Result summary, when it changes.
        #[serde(default)]
        result: Option<String>,
    },
    /// A task as its authority holds it, at a version that may skip the
    /// versions in between (a source that reads the whole task, not each
    /// update). An older version is refused; the same version may only
    /// change the flags and the places steps name. Conversation and assignee never change; the
    /// requester is set when a proposal is accepted. Legal transitions
    /// between the shown and the new state are not checked (the steps in
    /// between were not seen); the task's own consistency is.
    TaskSnapshot(Task),
    /// The source's answer to this desktop's request about a task (a
    /// decision on its way, or a refusal). Kept on the task until its
    /// version rises.
    TaskAnswered {
        /// Task.
        task_id: TaskId,
        /// The answer.
        answer: TaskAnswer,
    },
    /// The source no longer holds a task (its history was pruned): the
    /// task and the moments presenting it are dropped.
    TaskRemoved {
        /// Task.
        task_id: TaskId,
    },
    /// Source connectivity changed.
    SourceStatusChanged(SourceStatus),
    /// Local settings of a context changed. `None` fields are unchanged.
    ContextSettingsChanged {
        /// Context.
        context_id: ContextId,
        /// New pin state.
        #[serde(default)]
        pinned: Option<bool>,
        /// New mute state.
        #[serde(default)]
        muted: Option<bool>,
        /// New disappearing timer.
        #[serde(default)]
        disappearing: Option<Disappearing>,
    },
    /// An account was locked or unlocked.
    AccountLockChanged {
        /// Account.
        account_id: AccountId,
        /// New state.
        lock_state: LockState,
    },
    /// An account's app lock was read or changed (the whole state, so a
    /// refusal that closes it carries the policy as well).
    AppLockChanged {
        /// Account.
        account_id: AccountId,
        /// The lock as the daemon reports it.
        lock: AppLock,
    },
    /// The account code sheet of an app lock change opened (the daemon asked
    /// for the code) or closed (the change ended, or the code was answered).
    AppLockCodeSheet {
        /// Account.
        account_id: AccountId,
        /// Whether the sheet is open.
        open: bool,
    },
    /// The daemon refused the account code of an app lock change or unlock.
    AppLockCodeRefused {
        /// Account.
        account_id: AccountId,
        /// Why.
        refusal: crate::model::AppLockCodeRefusal,
    },
    /// The daemon answered a lifecycle report (the window went to the
    /// background or came to the front). Coming with the answer's
    /// [`DomainEvent::AppLockChanged`], it lets a window that holds its
    /// content for the answer to the latest foreground report tell that
    /// answer from any other app lock change.
    AppLockLifecycleAnswered {
        /// Account.
        account_id: AccountId,
        /// The phase the answered report named.
        phase: crate::intent::AppLockPhase,
    },
    /// Local node settings changed. Setting a default clears the previous one.
    NodeSettingsChanged {
        /// Node.
        node_id: NodeId,
        /// New enabled state.
        #[serde(default)]
        enabled: Option<bool>,
        /// Becomes the default node (only `Some(true)` is meaningful).
        #[serde(default)]
        is_default: Option<bool>,
    },
    /// A node probe finished.
    NodeProbed {
        /// Node.
        node_id: NodeId,
        /// Result.
        probe: NodeProbe,
    },
    /// A node profile was removed.
    NodeRemoved {
        /// Node.
        node_id: NodeId,
    },
    /// Device inserted or replaced (revocation keeps the record).
    DeviceUpserted(Device),
    /// The backend refused a decision on the grant, or the grant stopped
    /// being usable. The grant is closed; it is never retried from here.
    GrantFailed {
        /// Grant.
        grant_id: GrantId,
        /// Why.
        failure: GrantFailure,
    },
    /// An approved grant was revoked before use.
    GrantRevoked {
        /// Grant.
        grant_id: GrantId,
    },
    /// Standing approval inserted or replaced (revocation keeps the record).
    StandingApprovalUpserted(StandingApproval),
    /// One audit record appended.
    AuditAppended(AuditEntry),
    /// The read-only integrations snapshot was replaced.
    IntegrationsUpdated(Integrations),
    /// The backend declared which contract methods it cannot perform.
    CapabilitiesUpdated(Capabilities),
    /// The local daemon reported about itself.
    DaemonInfoUpdated(DaemonInfo),
    /// How the source is kept up to date changed (stream, polling, the
    /// daemon's background sync).
    SyncStatusChanged(SyncStatus),
    /// An effect could not be performed. Changes no domain entity.
    EffectFailed(EffectFailure),
    /// An effect of `method` succeeded: earlier failures of the same
    /// operation are no longer current.
    EffectSucceeded {
        /// Contract method of the effect.
        method: String,
    },
    /// The group invitations waiting for an answer (the whole list).
    GroupInvitationsListed(Vec<crate::GroupInvitation>),
    /// A group conversation is being created: its creation has not finished
    /// (`creating: true`), or it has (`false`, on success or failure). A
    /// creating group takes no messages yet.
    GroupCreation {
        /// The conversation being created.
        context_id: crate::ContextId,
        /// Whether the creation is still running.
        creating: bool,
    },
    /// This account's username, or why the directory cannot be used.
    UsernameStatusUpdated(crate::UsernameStatus),
    /// The answer to the last `@name` lookup made for `account` (`None`
    /// clears it). `None` as the account: a lookup made with no account open.
    UsernameLookupUpdated {
        /// The account the lookup was made for.
        account: Option<crate::AccountId>,
        /// The answer, or `None` to clear it.
        lookup: Option<crate::UsernameLookup>,
    },
    /// Progress of the last username claim or change of `account` (`None`
    /// clears it).
    UsernameClaimUpdated {
        /// The account whose name is being claimed or changed.
        account: crate::AccountId,
        /// The progress, or `None` to clear it.
        claim: Option<crate::UsernameClaim>,
    },
    /// A status read reported the claim operation `operation_id` of `account`
    /// as settled: committed, or aborted (nothing published).
    UsernameClaimSettled {
        /// The account whose claim settled.
        account: crate::AccountId,
        /// The directory operation the claim was submitted as.
        operation_id: String,
        /// Whether the directory committed the change.
        committed: bool,
    },
    /// An actionable networking failure associated with its originating account.
    AccountNetworkingFailed {
        /// Account that issued the operation.
        account_id: AccountId,
        /// Typed terminal failure.
        code: crate::FailureCode,
    },
    /// Whether this account's message traffic may flow (S10c D1/D7), and the
    /// schema generation the daemon reported, when it did.
    SchemaGateChanged {
        /// The account.
        account_id: AccountId,
        /// The verdict.
        gate: crate::SchemaGate,
        /// The generation the daemon reported.
        generation: Option<i64>,
    },
    /// The source answered these collections in full (each may be empty).
    ListingsLoaded(Vec<Listing>),
    /// This account's contact card, as the backend exported it.
    OwnContactCardUpdated(ContactCard),
    /// A voice call as the daemon reports it (the whole call). The version
    /// must rise, the state follow [`crate::CallState::can_transition`], and
    /// the conversation, peer, caller and direction stay. The store keeps
    /// the call's timeline record with it.
    CallUpserted(Call),
    /// The incoming contact requests the daemon lists now (pending ones):
    /// they replace every incoming request the store holds.
    ContactRequestsListed(Vec<ContactRequest>),
    /// One contact request, as it is now (a new one, an answer, an expiry,
    /// or this account's own request to a peer). Keyed by peer and
    /// direction: a peer has at most one request each way.
    ContactRequestUpserted(ContactRequest),
    /// The account's contact request setting.
    ContactRequestPolicyUpdated(RequestPolicy),
    /// The daemon's bound on remembered requester identities, as it is now
    /// (only a daemon that reports it sends this).
    ContactRequestFloorsUpdated(RequestFloors),
    /// The daemon has contact requests but could not list them now (a
    /// locked account, a transient outage, a row it could not read): the
    /// message for the user. The requests already known stay; the next
    /// listing clears it.
    ContactRequestsReadFailed(String),
    /// A full backup or restore job as the daemon reports it (the whole
    /// job). The version must rise, the kind and file stay, and the phase
    /// follow [`crate::BackupPhase::can_transition`]. Boxed: the job's
    /// review and report are large.
    BackupJobUpdated(Box<BackupJob>),
    /// The creation ceremony of a born-v3 account as the daemon reports it
    /// (the whole record, never the recovery phrase). It starts in
    /// `Prepared` (or `Failed` when the daemon refuses), then follows
    /// [`crate::CeremonyPhase::can_follow`]; the ceremony_id and expiry stay.
    CeremonyUpdated(Ceremony),
    /// The recovery ceremony of a born-v3 account as the daemon reports it
    /// (the whole record, never the phrase or the passphrase). It starts in
    /// `Prepared` (or `Failed` when the daemon refuses), then follows
    /// [`crate::RecoveryPhase::can_follow`]; the id and expiry stay.
    RecoveryUpdated(RecoveryCeremony),
    /// The daemon removed an account from this device (`account.remove_local`
    /// answered `removed: true`): the account and everything held for it
    /// leaves the store.
    AccountRemoved {
        /// The removed account.
        account_id: AccountId,
    },
    /// A page of agent records as the daemon lists them. Each record
    /// replaces the one held for its anchor.
    AgentsListed {
        /// The records of the page.
        records: Vec<AgentRecord>,
        /// The journal cursor the next watch starts from. Empty on a later
        /// page of a listing: the store keeps the cursor of the first page.
        journal_cursor: String,
    },
    /// The daemon refused the stored journal cursor (it is above its head):
    /// the cursor is dropped, and the roster is listed again.
    AgentCursorReset,
    /// One agent record as the daemon reports it (after a read, a watch or
    /// an edit). The record is replaced whole.
    AgentRecordUpdated(Box<AgentRecord>),
    /// The roster's read state: loading, ready, offline or failed.
    AgentRosterStateChanged(AgentRosterState),
    /// A creation request this desktop started, as the daemon reports it.
    /// Its state follows [`crate::AgentCreationState`].
    AgentCreationUpdated(AgentCreation),
    /// The PRX catalog as `agent.catalog` answered it.
    AgentCatalogLoaded(AgentCatalog),
    /// The configuration of one agent as `agent.get`, `agent.update`,
    /// `agent.pause` or `agent.resume` showed it, with its revision. A pause
    /// or a resume shows as `enabled`.
    AgentSettingsChanged {
        /// The agent's anchor.
        agent_anchor: String,
        /// The configuration now.
        settings: AgentSettings,
    },
    /// The PRX runtime status on this computer.
    AgentHostUpdated(AgentHostStatus),
    /// The PRX jobs of one agent, as the runtime lists them (the whole list).
    AgentJobsUpdated {
        /// The agent's anchor.
        agent: String,
        /// The jobs.
        jobs: Vec<AgentJob>,
    },
    /// The approvals of agents waiting for an answer (the whole list).
    AgentApprovalsUpdated(Vec<AgentApproval>),
    /// A call of an MLS group method was accepted by the source and is on
    /// its way (the join owner, the send owner and the gap guard record it).
    MlsStarted(MlsStart),
    /// A page of `group.list`. A first page keeps the rows until it arrives;
    /// the rows no page names are dropped when the last page is in.
    MlsGroupsListed {
        /// The rows of the page.
        rows: Vec<MlsGroupSummary>,
        /// The cursor of the next page; `None` at the end.
        cursor: Option<String>,
        /// Whether this is the first page of a listing.
        first_page: bool,
    },
    /// A typed failure of an MLS group method (the code only).
    MlsFailed {
        /// The contract method.
        method: String,
        /// The group, when the call named one.
        #[serde(default)]
        group_id: Option<String>,
        /// The invite, when the call named one.
        #[serde(default)]
        invite_id: Option<String>,
        /// The error code.
        code: String,
    },
    /// `group.info` of one group.
    MlsGroupInfoRead {
        /// The group as it is now.
        summary: Box<MlsGroupSummary>,
    },
    /// A page of `group.members`, or its failure.
    MlsMembersPage {
        /// The group.
        group_id: String,
        /// The rows.
        rows: Vec<MemberRow>,
        /// The cursor of the next page; `None` at the end.
        cursor: Option<String>,
        /// The error code when the page failed.
        #[serde(default)]
        failed: Option<String>,
    },
    /// A page of `group.history`.
    MlsHistoryPage {
        /// The group.
        group_id: String,
        /// How it was asked for.
        direction: HistoryDirection,
        /// The posts.
        posts: Vec<Post>,
        /// The first position this device can show.
        floor_log_seq: Option<u64>,
        /// The newest position.
        last_log_seq: u64,
        /// More rows in the direction asked.
        more: bool,
    },
    /// A `group.event` frame.
    MlsGroupEvent {
        /// The account log sequence of the frame.
        seq: u64,
        /// The group.
        group_id: String,
        /// The position the envelope names, when it does.
        #[serde(default)]
        log_seq: Option<u64>,
        /// What happened.
        kind: GroupEventKind,
    },
    /// `group.create` answered.
    MlsCreated {
        /// The new group.
        group_id: String,
        /// The name it was created with.
        name: String,
        /// Whether new members need approval.
        approval: bool,
        /// The first invite.
        invite: Invite,
    },
    /// `group.create` was refused.
    MlsCreateFailed {
        /// The error code.
        code: String,
    },
    /// An invite was created.
    MlsInviteCreated {
        /// The group.
        group_id: String,
        /// The invite.
        invite: Invite,
    },
    /// The invites this account created.
    MlsInvitesListed {
        /// The group.
        group_id: String,
        /// The invites.
        invites: Vec<Invite>,
    },
    /// An invite was revoked (the reply came).
    MlsInviteRevoked {
        /// The group.
        group_id: String,
        /// The invite.
        invite_id: String,
    },
    /// What one join attempt came to. A joined or pending outcome of an
    /// older attempt is recorded all the same.
    MlsJoinOutcome {
        /// The attempt.
        attempt: u64,
        /// The outcome.
        outcome: JoinOutcome,
    },
    /// The pending join requests the SDK holds (`group.join.pending`).
    MlsJoinPendingListed {
        /// The requests.
        records: Vec<JoinRecord>,
    },
    /// What a send came to.
    MlsSendOutcome {
        /// The group.
        group_id: String,
        /// The client id of the submit.
        local_id: String,
        /// The SDK's post id, when it answered.
        #[serde(default)]
        post_id: Option<String>,
        /// The state.
        state: SendState,
    },
    /// What a delete came to.
    MlsDeleteOutcome {
        /// The group.
        group_id: String,
        /// The post.
        post_id: String,
        /// The result.
        result: DeleteResult,
    },
    /// What a leave came to.
    MlsLeft {
        /// The group.
        group_id: String,
        /// The result.
        result: LeaveResult,
    },
    /// `group.refresh` answered for a group.
    MlsRefreshed {
        /// The group.
        group_id: String,
        /// The newest position the host reported.
        last_log_seq: u64,
    },
    /// `group.mark_read` answered.
    MlsMarkedRead {
        /// The group.
        group_id: String,
        /// The position read up to.
        read_log_seq: u64,
        /// Unread posts.
        unread: u32,
        /// Unread mentions.
        unread_mentions: u32,
    },
    /// A new method of the MLS groups is not available on this source.
    MlsUnavailable {
        /// The contract method.
        method: String,
        /// The group the call named.
        #[serde(default)]
        group_id: Option<String>,
        /// The join attempt the call carried.
        #[serde(default)]
        attempt: Option<u64>,
        /// The send the call carried.
        #[serde(default)]
        local_id: Option<String>,
    },
}

/// A call of an MLS group method that the source accepted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MlsStart {
    /// A join attempt (the link is not here).
    Join {
        /// The attempt.
        attempt: u64,
    },
    /// A send.
    Send {
        /// The group.
        group_id: String,
        /// The client id.
        local_id: String,
        /// The text.
        text: String,
        /// The reply target.
        #[serde(default)]
        reply_to: Option<String>,
        /// Mentioned anchors.
        #[serde(default)]
        mentions: Vec<String>,
        /// The `NotSent` bubble this send replaces.
        #[serde(default)]
        retry_of: Option<String>,
    },
    /// A history read.
    History {
        /// The group.
        group_id: String,
        /// The direction.
        direction: HistoryDirection,
    },
    /// A refresh.
    Refresh {
        /// The group.
        group_id: String,
    },
    /// A delete.
    Delete {
        /// The group.
        group_id: String,
        /// The post.
        post_id: String,
    },
    /// A leave.
    Leave {
        /// The group.
        group_id: String,
    },
    /// A create.
    Create,
    /// A page of the group list.
    List,
    /// A read of one group.
    Info {
        /// The group.
        group_id: String,
    },
}

impl DomainEvent {
    /// The `MlsStarted` fact for an accepted call of an MLS group method,
    /// read from the call body the policy built. A source emits it when it
    /// takes the call, before any answer, so the join owner, the send owner
    /// and the gap guard know what is on its way. `None` for a method that
    /// needs no such record.
    pub fn mls_started(method: &str, body: &JsonValue) -> Option<Self> {
        use crate::intent::methods as m;
        let text = |field: &str| body.get(field).and_then(JsonValue::as_str).map(str::to_owned);
        let group = || text("group_id");
        let start = match method {
            m::MLS_GROUP_JOIN => MlsStart::Join { attempt: body.get("attempt")?.as_u64()? },
            m::MLS_GROUP_SEND => MlsStart::Send {
                group_id: group()?,
                local_id: text("local_id")?,
                text: text("text")?,
                reply_to: text("reply_to"),
                mentions: body
                    .get("mentions")
                    .and_then(JsonValue::as_array)
                    .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
                    .unwrap_or_default(),
                retry_of: text("retry_of"),
            },
            m::MLS_GROUP_HISTORY => {
                let direction = if let Some(ls) = body.get("before_log_seq").and_then(JsonValue::as_u64) {
                    HistoryDirection::Before(ls)
                } else if let Some(ls) = body.get("after_log_seq").and_then(JsonValue::as_u64) {
                    HistoryDirection::After(ls)
                } else {
                    HistoryDirection::Newest
                };
                MlsStart::History { group_id: group()?, direction }
            }
            m::MLS_GROUP_REFRESH => MlsStart::Refresh { group_id: group()? },
            m::MLS_GROUP_MESSAGE_DELETE => {
                MlsStart::Delete { group_id: group()?, post_id: text("post_id")? }
            }
            m::MLS_GROUP_LEAVE => MlsStart::Leave { group_id: group()? },
            m::MLS_GROUP_CREATE => MlsStart::Create,
            m::MLS_GROUP_LIST => MlsStart::List,
            m::MLS_GROUP_INFO => MlsStart::Info { group_id: group()? },
            _ => return None,
        };
        Some(Self::MlsStarted(start))
    }
}
