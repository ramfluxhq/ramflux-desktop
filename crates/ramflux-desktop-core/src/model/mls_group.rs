// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! MLS groups (method table r4): summaries, members, invites, joins, posts
//! and the per-group timeline the store keeps in memory.
//!
//! Nothing here is persisted. Invite links are secrets: [`InviteLink`] and
//! [`Invite`] print `link=<redacted>`, and the anchors of members never
//! print. Enums that the wire can extend carry an `Unknown(String)` case,
//! except the two that live inside `Copy` receipts ([`QueueReason`] and
//! [`NotSentReason`]), whose unknown case carries no text.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ids::{ContextId, Timestamp};

/// Prefix of the context id of an MLS group (`mls:<group_id>`).
pub const CONTEXT_PREFIX: &str = "mls:";
/// Prefix of the participant ids standing for authors of MLS groups.
pub const PARTICIPANT_PREFIX: &str = "mls-p-";
/// Posts kept in the window of one timeline.
pub const WINDOW: usize = 600;
/// Timelines kept (the most recently opened groups).
pub const KEPT_TIMELINES: usize = 3;
/// Names a collapsed membership line carries.
pub const MAX_NAMES: usize = 500;
/// Longest invite link, in characters.
pub const MAX_LINK_LEN: usize = 512;
/// Longest group name, in characters.
pub const MAX_NAME_LEN: usize = 64;
/// Smallest gap between two `group.refresh` calls for one group.
pub const REFRESH_COALESCE_MS: i64 = 5_000;
/// Mentions one post may carry.
pub const MAX_MENTIONS: usize = 50;

/// The context id of the group `group_id`.
pub fn context_id(group_id: &str) -> ContextId {
    ContextId::new(format!("{CONTEXT_PREFIX}{group_id}"))
}

/// The group a context id names, when it is an MLS group context.
pub fn group_of(context: &ContextId) -> Option<&str> {
    context.as_str().strip_prefix(CONTEXT_PREFIX)
}

/// An invite link: a secret. It never prints and is passed on verbatim.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct InviteLink(String);

impl InviteLink {
    /// Wraps the link as typed (no check).
    pub fn new(link: impl Into<String>) -> Self {
        Self(link.into())
    }

    /// The link text, to send or to copy on an explicit tap.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for InviteLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("link=<redacted>")
    }
}

/// The local check of a typed or scanned link (S2): ASCII whitespace at the
/// ends is trimmed, and what is left is 1 to 512 printable ASCII characters
/// (0x21 to 0x7E). The link is otherwise passed on verbatim.
pub fn check_link(typed: &str) -> Option<InviteLink> {
    let trimmed = typed.trim_matches(|c: char| c.is_ascii_whitespace());
    let valid = !trimmed.is_empty()
        && trimmed.len() <= MAX_LINK_LEN
        && trimmed.bytes().all(|b| (0x21..=0x7E).contains(&b));
    valid.then(|| InviteLink::new(trimmed))
}

/// Whether `name` is a valid group name once trimmed (1 to 64 characters).
pub fn check_name(name: &str) -> bool {
    let count = name.trim().chars().count();
    (1..=MAX_NAME_LEN).contains(&count)
}

/// A role in a group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MlsRole {
    /// The owner.
    Owner,
    /// An administrator.
    Admin,
    /// A member.
    Member,
    /// A role this build does not know.
    Unknown(String),
}

/// The notification mode of this device for a group (read-only in M2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PushSetting {
    /// Mentions and replies.
    MentionsReplies,
    /// Every message.
    All,
    /// None.
    None,
    /// A mode this build does not know.
    Unknown(String),
}

/// A member as a post, a service line or a roster row names it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Author {
    /// The account root commitment: opaque, never shown or logged.
    pub anchor: String,
    /// The display name; `None` shows the localized "Member".
    pub display: Option<String>,
    /// The SDK's AI flag.
    pub ai_flag: bool,
    /// Whether this is the local account.
    pub is_me: bool,
}

impl fmt::Debug for Author {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Author {{ ai_flag: {}, is_me: {} }}", self.ai_flag, self.is_me)
    }
}

/// A sending or commit allowance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quota {
    /// The limit of the bucket.
    pub limit: u64,
    /// What is left.
    pub remaining: u64,
    /// Whether the first-day limit applies.
    pub first_day: bool,
    /// When the bucket refills.
    pub resets_at: Option<Timestamp>,
}

/// The privacy delay of a group (parsed in M2, shown in M3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrivacyDelay {
    /// None.
    None,
    /// Armed, no send time yet.
    Armed,
    /// A post waits until then.
    Waiting(Timestamp),
}

/// A warning of a group (only `host_ip_visible` is shown in M2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Warning {
    /// The code.
    pub code: String,
    /// Its parameters.
    pub params: Value,
    /// Whether the warning was cleared.
    pub cleared: bool,
}

/// The catch-up state of a group (spinner only in M2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatchUp {
    /// The state token.
    pub state: String,
    /// Done.
    pub done: u64,
    /// Total.
    pub total: u64,
}

/// The last post of a group, for the list row (text only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastPost {
    /// Its position.
    pub log_seq: u64,
    /// Its kind token.
    pub kind: String,
    /// Who wrote it.
    pub author: Option<Author>,
    /// The first 100 characters of a text post.
    pub snippet: Option<String>,
    /// When it was appended.
    pub appended_at: Option<Timestamp>,
}

/// A group as `group.list` and `group.info` describe it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MlsGroupSummary {
    /// The group id.
    pub group_id: String,
    /// Its name.
    pub name: String,
    /// The size tier.
    pub tier: u32,
    /// Members.
    pub members: u32,
    /// Members that left.
    pub leaves: u32,
    /// Agents among the members.
    pub agents: u32,
    /// Whether new members need approval.
    pub approval: bool,
    /// This account's role.
    pub my_role: MlsRole,
    /// The host node.
    pub host_node: String,
    /// The notification mode of this device.
    pub push: PushSetting,
    /// Catch-up state, when running.
    pub catch_up: Option<CatchUp>,
    /// The privacy delay.
    pub privacy_delay: PrivacyDelay,
    /// The post quota.
    pub quota_post: Option<Quota>,
    /// The commit quota.
    pub quota_commit: Option<Quota>,
    /// The newest position this device knows.
    pub last_log_seq: u64,
    /// Warnings.
    pub warnings: Vec<Warning>,
    /// The last activity (list rows only).
    pub last_activity_at: Option<Timestamp>,
    /// Unread posts.
    pub unread: u32,
    /// Unread mentions of or replies to this account.
    pub unread_mentions: u32,
    /// The position read up to.
    pub read_log_seq: u64,
    /// The last post.
    pub last_post: Option<LastPost>,
}

impl MlsGroupSummary {
    /// Whether the group host sees this device's network address.
    pub fn host_ip_visible(&self) -> bool {
        self.warnings.iter().any(|w| w.code == "host_ip_visible" && !w.cleared)
    }
}

/// A row of the member roster.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberRow {
    /// The anchor: opaque, never shown or logged.
    pub anchor: String,
    /// The display name; `None` shows "Member".
    pub display: Option<String>,
    /// The role.
    pub role: MlsRole,
    /// The SDK's AI flag.
    pub ai_flag: bool,
    /// Devices of the member.
    pub devices: u32,
    /// Whether this is the local account.
    pub is_me: bool,
}

impl fmt::Debug for MemberRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MemberRow {{ role: {:?}, ai_flag: {} }}", self.role, self.ai_flag)
    }
}

/// An invite of a group.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invite {
    /// Its id.
    pub invite_id: String,
    /// The link: a secret.
    pub link: InviteLink,
    /// Most uses.
    pub max_uses: u32,
    /// Uses left; known right after creation only.
    pub uses_left: Option<u32>,
    /// When it expires.
    pub expires_at: Timestamp,
    /// Whether it was revoked.
    pub revoked: bool,
}

impl fmt::Debug for Invite {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Invite { link=<redacted> }")
    }
}

/// Why a join waits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PendingReason {
    /// An admin has to approve.
    AwaitingAdmin,
    /// A member has to come online.
    AwaitingMemberOnline,
    /// A reason this build does not know.
    Unknown(String),
}

/// Why a join was refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum JoinRefusal {
    /// The link is not a group invite.
    LinkInvalid,
    /// The link expired (decided locally from the link).
    InviteExpired {
        /// When.
        expired_at: Option<Timestamp>,
    },
    /// The invite does not work (unknown, revoked, used up or stale).
    InviteInvalid,
    /// The group is full.
    GroupFull {
        /// The leaf cap.
        leaf_cap: Option<u32>,
    },
    /// Too many people wait to join.
    QueueFull,
    /// The request was not approved in time.
    RequestExpired {
        /// When it expired.
        expired_at: Option<Timestamp>,
    },
    /// The account is in the group already.
    AlreadyMember {
        /// The group.
        group_id: Option<String>,
    },
    /// Too many attempts.
    RateLimited {
        /// Seconds to wait.
        retry_after_s: Option<u64>,
    },
    /// A reason this build does not know.
    Unknown(String),
}

/// What one join attempt came to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum JoinOutcome {
    /// Joined.
    Joined {
        /// The group.
        group_id: String,
    },
    /// Waiting for approval.
    Pending {
        /// The request.
        request_id: String,
        /// Why it waits.
        reason: PendingReason,
        /// When the request expires.
        expires_at: Option<Timestamp>,
        /// The group, when known.
        group_id: Option<String>,
    },
    /// Refused.
    Refused(JoinRefusal),
    /// The call failed (a typed error code).
    Failed {
        /// The code.
        code: String,
    },
    /// Groups are not available in this build or on this source.
    Unavailable,
}

/// A pending join request, held in memory only. It never holds the link.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinRecord {
    /// The request id.
    pub request_id: String,
    /// Why it waits.
    pub reason: PendingReason,
    /// When it expires.
    pub expires_at: Option<Timestamp>,
    /// The group, when known.
    pub group_id: Option<String>,
    /// The attempt that made it (0 when read back after a relaunch).
    pub attempt: u64,
}

impl fmt::Debug for JoinRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "JoinRecord {{ attempt: {} }}", self.attempt)
    }
}

/// The join owner of the account: the attempt counter, the call in flight,
/// the last outcome of each attempt and the pending requests.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JoinState {
    /// The highest attempt number seen.
    pub last_attempt: u64,
    /// The attempt whose call is in flight.
    pub in_flight: Option<u64>,
    /// The latest outcome, with its attempt.
    pub last: Option<(u64, JoinOutcome)>,
    /// Pending requests by request id.
    pub records: BTreeMap<String, JoinRecord>,
}

/// The kind of a post.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PostKind {
    /// Text.
    Text,
    /// An attachment.
    Attachment,
    /// A card.
    Card,
    /// A membership change.
    Membership,
    /// A settings change.
    Settings,
    /// An undone change.
    Void,
    /// A kind this build does not know.
    Unknown(String),
}

/// The state of a post.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PostState {
    /// In the log.
    Sent,
    /// Own, not in the log yet.
    Pending,
    /// Queued by the SDK.
    Queued,
    /// Delayed for privacy.
    DelayedPrivacy,
    /// Rate limited.
    RateLimited,
    /// Failed.
    Failed,
    /// Deleted.
    Deleted,
    /// A state this build does not know.
    Unknown(String),
}

/// What happened to a member in a membership row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MembershipKind {
    /// Joined.
    Joined,
    /// Left.
    Left,
    /// Removed.
    Removed,
    /// Banned.
    Banned,
    /// Role changed.
    RoleChanged,
    /// A device was added (never shown).
    DeviceAdded,
    /// A device was removed (never shown).
    DeviceRemoved,
    /// A change this build does not know.
    Unknown(String),
}

/// One change of a membership row.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MembershipChange {
    /// What happened.
    pub kind: MembershipKind,
    /// To whom (anchor: never shown or logged).
    pub anchor: String,
    /// The display name; `None` shows "Member".
    pub display: Option<String>,
    /// The SDK's AI flag.
    pub ai_flag: bool,
    /// The new role of a role change.
    pub role: Option<MlsRole>,
    /// Whether the change is about the local account.
    pub is_me: bool,
}

impl fmt::Debug for MembershipChange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MembershipChange {{ kind: {:?} }}", self.kind)
    }
}

/// What a settings row changed (the notification mode is per device and
/// not a group setting).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsChange {
    /// The new name.
    pub name: Option<String>,
    /// The new approval mode.
    pub approval: Option<bool>,
}

/// The body of a post.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PostBody {
    /// Text.
    Text(String),
    /// A membership row: up to 100 changes of one commit.
    Membership {
        /// The changes.
        changes: Vec<MembershipChange>,
        /// Who did it, when somebody did.
        by: Option<Author>,
    },
    /// A settings row.
    Settings {
        /// What changed.
        change: SettingsChange,
        /// Who did it.
        by: Option<Author>,
    },
    /// An undone change.
    Void {
        /// The position undone.
        voided_log_seq: Option<u64>,
        /// Who did it.
        by: Option<Author>,
    },
    /// An attachment, a card or a kind this build does not know.
    Opaque,
}

/// A post as `group.history` and the `post` event carry it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Post {
    /// Its position in the group log; `None` while it is not in the log.
    pub log_seq: Option<u64>,
    /// Its id.
    pub post_id: String,
    /// Who wrote it (a service row has no author: the store then reads
    /// the actor from `by`).
    pub author: Option<Author>,
    /// When it was appended.
    pub appended_at: Option<Timestamp>,
    /// Its kind.
    pub kind: PostKind,
    /// Its body.
    pub body: PostBody,
    /// The post it replies to.
    pub reply_to: Option<String>,
    /// Mentioned anchors.
    pub mentions: Vec<String>,
    /// Its state.
    pub state: PostState,
    /// For an own post not in the log: how the SDK holds it.
    pub send: Option<SendState>,
}

/// Why a post waits in the SDK queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QueueReason {
    /// The device catches up.
    CatchingUp,
    /// The group host cannot be reached.
    HostUnreachable,
    /// Tokens are fetched.
    FetchingTokens,
    /// A reason this build does not know.
    Unknown,
}

/// Why a post was not sent. Nothing is resent by itself: only Retry sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NotSentReason {
    /// The limit was reached.
    RateLimited {
        /// When the bucket refills.
        resets_at: Option<Timestamp>,
        /// Whether the first-day limit applies.
        first_day: bool,
        /// The limit.
        limit: u64,
        /// What is left.
        remaining: u64,
    },
    /// Too large.
    TooLarge {
        /// The limit in bytes.
        max_bytes: Option<u64>,
    },
    /// Not a member of the group.
    NotMember,
    /// This device has to rejoin.
    RejoinNeeded,
    /// The group host cannot be reached.
    HostUnreachable,
    /// Sending is paced until then.
    Paced {
        /// Until when.
        until: Option<Timestamp>,
    },
    /// An error code this build does not know.
    Unknown,
    /// The SDK reported the post failed (from an event).
    Failed,
}

/// The state of an own post on its way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SendState {
    /// The call is running.
    Submitting,
    /// In the log.
    Sent {
        /// Its position.
        log_seq: u64,
    },
    /// Queued by the SDK, which sends it by itself.
    Queued(QueueReason),
    /// Delayed for privacy; the SDK sends it around then.
    Delayed {
        /// When.
        send_after: Timestamp,
    },
    /// Not sent.
    NotSent(NotSentReason),
}

/// How a history page was asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HistoryDirection {
    /// The newest page.
    Newest,
    /// Posts before a position.
    Before(u64),
    /// Posts after a position.
    After(u64),
}

/// The kinds of `group.event` this build handles (others are skipped).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GroupEventKind {
    /// A post.
    Post(Box<Post>),
    /// A post changed state.
    PostState {
        /// The post.
        post_id: String,
        /// Its new state.
        state: SendState,
    },
    /// A post was deleted.
    PostDeleted {
        /// The post.
        post_id: String,
        /// Its position in the log.
        post_log_seq: Option<u64>,
        /// The position of the delete record.
        log_seq: Option<u64>,
    },
    /// A membership row.
    Membership(Box<Post>),
    /// A settings row.
    Settings(Box<Post>),
    /// A pending join request ended.
    JoinResult {
        /// The request.
        request_id: String,
        /// The group.
        group_id: String,
        /// Joined, or the refusal.
        result: Result<(), JoinRefusal>,
    },
    /// The read position moved.
    Read {
        /// The position.
        read_log_seq: u64,
        /// Unread posts.
        unread: u32,
        /// Unread mentions.
        unread_mentions: u32,
    },
}

/// How a delete ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeleteResult {
    /// The call runs.
    Started,
    /// The delete record is in the log.
    Committed {
        /// Its position.
        log_seq: u64,
    },
    /// The SDK holds the delete and sends it by itself.
    Queued,
    /// The SDK delays the delete for privacy.
    Delayed,
    /// Refused or failed (a typed error code).
    Failed {
        /// The code.
        code: String,
    },
}

/// How a leave ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LeaveResult {
    /// Left.
    Committed,
    /// The owner cannot leave.
    OwnerProtected,
    /// Failed (a typed error code).
    Failed {
        /// The code.
        code: String,
    },
}

/// Why a group takes no messages from this account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReadOnly {
    /// The account left.
    Left,
    /// The account was removed.
    Removed,
    /// The account was banned.
    Banned,
}

/// A name in a service line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MlsName {
    /// The display name; `None` shows "Member".
    pub display: Option<String>,
    /// Whether it is the local account ("You").
    pub is_me: bool,
}

/// A service line of an MLS group timeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MlsServiceLine {
    /// Members joined (a collapsed run, up to 500 names).
    Joined {
        /// Who.
        who: Vec<MlsName>,
    },
    /// Members left.
    Left {
        /// Who.
        who: Vec<MlsName>,
    },
    /// A member was removed.
    Removed {
        /// By whom.
        by: Option<MlsName>,
        /// Who.
        target: MlsName,
    },
    /// A member was banned.
    Banned {
        /// By whom.
        by: Option<MlsName>,
        /// Who.
        target: MlsName,
    },
    /// A role changed to admin or member.
    RoleChanged {
        /// By whom.
        by: Option<MlsName>,
        /// Who.
        target: MlsName,
        /// Admin when true, member when false.
        admin: bool,
    },
    /// The group was renamed.
    Renamed {
        /// By whom.
        by: Option<MlsName>,
        /// The new name.
        name: String,
    },
    /// Approval was switched.
    Approval {
        /// By whom.
        by: Option<MlsName>,
        /// The new mode.
        on: bool,
    },
    /// A change was undone.
    Void,
    /// A change this build does not show.
    Updated,
    /// A deleted message.
    Deleted,
    /// Earlier messages are not available on this device.
    Floor,
}

/// An own post that is not in the log, or a post the user is sending.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnItem {
    /// The client-made id of the submit.
    pub local_id: String,
    /// The SDK's id, once it answered.
    pub post_id: Option<String>,
    /// The text, kept in memory for Retry.
    pub text: String,
    /// The reply target.
    pub reply_to: Option<String>,
    /// Mentioned anchors.
    pub mentions: Vec<String>,
    /// How it stands.
    pub send: SendState,
    /// When it was submitted.
    pub at: Timestamp,
    /// Send order.
    pub order: u64,
}

/// The in-memory window of one group's posts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MlsTimeline {
    /// Posts in the log by position.
    pub posted: BTreeMap<u64, Post>,
    /// Own posts that are not in the log, in send order.
    pub own: Vec<OwnItem>,
    /// The first position this device can show.
    pub floor_log_seq: Option<u64>,
    /// Whether older rows exist.
    pub more_older: bool,
    /// Whether the newest page was loaded.
    pub loaded: bool,
    /// The newest position known.
    pub last_log_seq: u64,
    /// Rows at the newest end were dropped: events no longer extend the window.
    pub tail_cut: bool,
    /// A gap fill runs.
    pub gap_in_flight: bool,
    /// Another round follows the running one.
    pub gap_pending: bool,
    /// A page of older rows was asked for and has not come yet.
    pub older_in_flight: bool,
    /// A gap was seen while a gap fill ran (one more round follows).
    pub gap_dirty: bool,
    /// Own posts being deleted.
    pub deleting: std::collections::BTreeSet<String>,
    /// Use counter, for keeping the most recent timelines.
    pub used: u64,
    /// Counter of own posts, for send order.
    pub next_order: u64,
}

/// The members of a group as far as they are loaded.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemberPages {
    /// Rows by anchor.
    pub rows: BTreeMap<String, MemberRow>,
    /// The cursor of the next page; `None` at the end.
    pub cursor: Option<String>,
    /// Whether the first page arrived.
    pub loaded: bool,
    /// The error code of the page that failed last (cleared by the next
    /// page that arrives).
    pub failed: Option<String>,
}

/// Where the group list stands.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ListPhase {
    /// Never asked.
    #[default]
    Idle,
    /// The first page is on its way.
    Loading,
    /// At least one page arrived.
    Ready,
    /// The read failed (the code).
    Failed(String),
}

/// The group list as far as it is loaded.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListState {
    /// Where it stands.
    pub phase: ListPhase,
    /// The cursor of the next page; `None` at the end (or before the first).
    pub cursor: Option<String>,
    /// Whether the last page arrived (the cursor ended).
    pub complete: bool,
}

/// The create owner of the account.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CreateState {
    /// A create call is in flight.
    pub in_flight: bool,
    /// Counts the outcomes, so a dialog tells a new outcome from an old one.
    pub seq: u64,
    /// The latest outcome: the new group, or the error code.
    pub outcome: Option<Result<String, String>>,
}
