// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Named agents of an owner (AGENT-IDENTITY-01), as the desktop sees them.
//!
//! The daemon owns the records: the desktop keeps the last record it was
//! given, the state of the creation it started, the runtime (PRX) status
//! and the jobs and approvals of the runtime. Nothing here holds a provider
//! key, a credential or a passphrase. Everything the core or PRX cannot do
//! yet is a typed [`AgentUnavailable`] reason, never a silent default.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The persona of an agent: the name, the avatar preset token and the
/// responsibility the owner gave it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentPersona {
    /// Display name.
    pub name: String,
    /// Avatar token: a preset, `preset:<glyph>:<tone>`, never an image.
    pub avatar: String,
    /// What the agent is for.
    pub responsibility: String,
}

/// A closed access token for direct messages or group invitations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentAccessMode {
    /// Only the owner.
    Owner,
    /// The owner's contacts.
    Contacts,
    /// The humans listed in `allowed_humans`.
    Allowlist,
    /// Anyone who can reach the agent.
    Open,
}

/// Who may reach an agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentAccess {
    /// Who may start a direct conversation.
    pub dm: AgentAccessMode,
    /// Who may invite the agent into a group.
    pub group_inviters: AgentAccessMode,
    /// Human anchors allowed, only when one mode is `allowlist`.
    pub allowed_humans: Vec<String>,
}

impl AgentAccess {
    /// The access the daemon takes for a new agent: owner only.
    pub fn owner_only() -> Self {
        Self {
            dm: AgentAccessMode::Owner,
            group_inviters: AgentAccessMode::Owner,
            allowed_humans: Vec::new(),
        }
    }

    /// Whether the access is one the daemon accepts: an allowlist names
    /// humans, and no other mode does.
    pub fn is_valid(&self) -> bool {
        let allowlist = self.dm == AgentAccessMode::Allowlist
            || self.group_inviters == AgentAccessMode::Allowlist;
        let unique = self
            .allowed_humans
            .iter()
            .enumerate()
            .all(|(i, anchor)| !self.allowed_humans[..i].contains(anchor));
        unique && (allowlist || self.allowed_humans.is_empty())
    }
}

/// The lifecycle of a record: `U` active, `X` retired (the primary's
/// action; this desktop never retires).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentLifecycle {
    /// State `U`.
    Active,
    /// State `X`: greyed out, read only.
    Retired,
    /// A state this desktop does not know.
    Other,
}

impl AgentLifecycle {
    /// Reads the daemon's lifecycle code.
    pub fn from_code(code: &str) -> Self {
        match code {
            "U" => Self::Active,
            "X" => Self::Retired,
            _ => Self::Other,
        }
    }
}

/// One agent record as the daemon reports it. `base` is the identity the
/// daemon returns with the record; an access change must send it back
/// unchanged (its `stale_base` refusal means: read again).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentRecord {
    /// The agent's anchor (its identity on the network).
    pub agent_anchor: String,
    /// The owner's human anchor.
    pub custodian_anchor: String,
    /// The persona.
    pub persona: AgentPersona,
    /// The lifecycle.
    pub lifecycle: AgentLifecycle,
    /// Who may reach it.
    pub access: AgentAccess,
    /// The role; `named` is the only role at this core rev.
    pub role: String,
    /// The record's identity, as the daemon gave it (opaque JSON).
    pub base: serde_json::Value,
    /// The name PRX knows the agent by (derived from the anchor), or empty
    /// when the anchor is not a valid one. An approval names its agent by
    /// this name.
    #[serde(default)]
    pub runtime_name: String,
    /// The configuration the daemon mirrors for the agent, with the
    /// per-agent revision a change must be sent with; `None` until it is
    /// read.
    #[serde(default)]
    pub settings: Option<AgentSettings>,
}

/// An agent's configuration as `agent.get` and `agent.update` show it. The
/// revision is the per-agent one: a change names the one the screen was
/// rendered from, and the daemon refuses it as `revision_conflict` when the
/// configuration moved on.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSettings {
    /// The provider id.
    pub provider: String,
    /// The model id.
    pub model: String,
    /// The autonomy token (`full`, `supervised` or `readonly`).
    pub autonomy: String,
    /// The prompt; `None` is the one PRX composes.
    pub prompt: Option<String>,
    /// The tool names; empty or `*` is every tool.
    pub tools: Vec<String>,
    /// Whether the agent runs (false: paused).
    pub enabled: bool,
    /// Concurrent turns allowed, when limited.
    pub max_concurrent_turns: Option<u32>,
    /// Cost allowed per day in cents, when limited.
    pub max_cost_per_day_cents: Option<u64>,
    /// The per-agent revision.
    pub revision: Option<String>,
}

/// What a change of the prompt does.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PromptChange {
    /// The prompt stays as it is.
    #[default]
    Keep,
    /// The prompt goes back to the one PRX composes.
    Reset,
    /// The prompt becomes this text.
    Set(String),
}

/// What a change of the limits does.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LimitsChange {
    /// The limits stay as they are.
    #[default]
    Keep,
    /// No limit.
    Clear,
    /// Replace both limits (an unset one is unlimited).
    Set {
        /// Concurrent turns allowed.
        max_concurrent_turns: Option<u32>,
        /// Cost allowed per day, in cents.
        max_cost_per_day_cents: Option<u64>,
    },
}

/// The fields an `agent.update` changes; a field left out stays as it is.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentChange {
    /// The provider.
    pub provider: Option<String>,
    /// The model.
    pub model: Option<String>,
    /// The autonomy token.
    pub autonomy: Option<String>,
    /// The prompt.
    pub prompt: PromptChange,
    /// The tool names (empty: every tool).
    pub tools: Option<Vec<String>>,
    /// The limits.
    pub limits: LimitsChange,
}

/// One model the PRX catalog offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogModel {
    /// The provider id.
    pub provider: String,
    /// The model id.
    pub model: String,
    /// Whether the provider's key is held: without it the model is shown
    /// disabled ("Add the provider key first").
    pub credentials_ready: bool,
}

/// One tool the PRX catalog offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogTool {
    /// The tool's name.
    pub name: String,
    /// Whether it only reads.
    pub read_only: bool,
}

/// The PRX catalog (`agent.catalog`): what an agent may be set to.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentCatalog {
    /// The PRX version that answered.
    pub prx_version: String,
    /// PRX's own revision of the catalog.
    pub revision: String,
    /// The models.
    pub models: Vec<CatalogModel>,
    /// The tools.
    pub tools: Vec<CatalogTool>,
    /// The autonomy a new agent starts with.
    pub default_autonomy: String,
    /// Names no agent may take.
    pub reserved_names: Vec<String>,
}

impl AgentCatalog {
    /// Whether `name` is one no agent may take (case-insensitive).
    #[must_use]
    pub fn reserves(&self, name: &str) -> bool {
        let name = name.trim();
        self.reserved_names.iter().any(|reserved| reserved.eq_ignore_ascii_case(name))
    }
}

impl AgentRecord {
    /// Whether an approval's `agent` text names this agent: its anchor or
    /// its runtime name.
    #[must_use]
    pub fn is_named(&self, agent: &str) -> bool {
        !agent.is_empty()
            && (self.agent_anchor == agent
                || (!self.runtime_name.is_empty() && self.runtime_name == agent))
    }
}

/// Why a request or a runtime action was refused, typed. The codes are the
/// daemon's and PRX's; `Other` keeps a code this desktop has no wording for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentRefusal {
    /// The daemon needs the owner's confirmation.
    ConfirmationRequired,
    /// This device is the account's primary: it cannot create agents.
    DesktopRequired,
    /// The desktop is not linked to the phone yet.
    CreationPairingRequired,
    /// The name, responsibility, provider or model is empty.
    MetadataPending,
    /// The access policy is not one the daemon accepts.
    AccessPolicyInvalid,
    /// The persona is not one the daemon accepts.
    PersonaInvalid,
    /// The record changed since it was read; read it again.
    StaleBase,
    /// The configuration changed since the screen was rendered
    /// (`revision_conflict`): read it again and review.
    RevisionConflict,
    /// The operation is not offered for this record (for example, delete
    /// of a retired agent).
    Unsupported,
    /// The daemon runs without PRX.
    PrxUnconfigured,
    /// The daemon does not take a host binding.
    HostNotSupported,
    /// PRX is not installed on this computer.
    PrxMissing,
    /// A `prx` was found only where this computer does not trust it (a
    /// relative PATH entry, or a folder or file others can write or another
    /// user owns).
    PrxPathUnsafe,
    /// The `prx` that would run is not the one the owner confirmed (the
    /// search path or the file changed meanwhile).
    PrxPathChanged,
    /// No provider was chosen.
    ProviderMissing,
    /// The provider credential is not available to PRX.
    CredentialUnavailable,
    /// The PRX runtime is older than this desktop expects.
    RuntimeStale,
    /// The PRX host rejected an ancestor directory.
    AncestorRejected,
    /// The signed PRX manifest is not available yet.
    ManifestUnavailable,
    /// This platform has no PRX build.
    UnsupportedPlatform,
    /// PRX reported a command failure.
    PrxCommand,
    /// PRX could not be started.
    PrxSpawn,
    /// PRX refused the request and asked for a confirmation the desktop
    /// cannot send yet (PRX answers 428).
    PrxRejected,
    /// PRX could not be reached (or it timed out).
    PrxUnavailable,
    /// PRX rejected the credential of this runtime (a token from another
    /// process, until the runtime restarts).
    AuthFailed,
    /// PRX does not support the request.
    PrxUnsupported,
    /// The PRX cron job named does not exist.
    CronJobUnknown,
    /// The PRX xin task named does not exist.
    XinTaskUnknown,
    /// The PRX xin goal named does not exist.
    XinGoalUnknown,
    /// PRX rejected a field of the request.
    FieldInvalid,
    /// PRX rejected the agent the request names.
    AgentInvalid,
    /// PRX could not use its storage.
    StorageUnavailable,
    /// The daemon or PRX did not answer in time.
    Timeout,
    /// The connection to the daemon broke.
    Transport,
    /// A code this desktop has no wording for.
    Other(String),
}

impl AgentRefusal {
    /// Whether a restart of the PRX runtime may clear this refusal (an
    /// expired token or an unreachable runtime), so the UI offers it.
    pub fn offers_restart(&self) -> bool {
        matches!(self, Self::AuthFailed | Self::PrxUnavailable)
    }
}

/// The state of one creation request, from the desktop's point of view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentCreationState {
    /// Written, not sent yet.
    Draft,
    /// Sent: waiting for the owner's phone to approve or deny.
    PendingPhone,
    /// Approved; the daemon is publishing the agent.
    Confirming,
    /// The agent exists and runs.
    Created,
    /// The phone denied it.
    Denied,
    /// Nobody answered before the expiry.
    Expired,
    /// Cancelled before it finished.
    Cancelled,
    /// Failed, with the typed reason.
    Failed(AgentRefusal),
}

impl AgentCreationState {
    /// Maps the daemon's custody state. An unknown state is a failure, so
    /// the desktop never shows success it did not see.
    pub fn from_custody_state(state: &str) -> Self {
        match state {
            "authorization_pending" => Self::PendingPhone,
            "authorized" | "publication_pending" | "publication_unknown" => Self::Confirming,
            "complete" | "live" => Self::Created,
            "authorization_refused" | "refused" => Self::Denied,
            "expired" => Self::Expired,
            "cancelled" => Self::Cancelled,
            other => Self::Failed(AgentRefusal::Other(other.to_owned())),
        }
    }

    /// Whether the desktop stops polling the request.
    pub fn is_terminal(&self) -> bool {
        !matches!(self, Self::Draft | Self::PendingPhone | Self::Confirming)
    }
}

/// One creation request this desktop started (kept until terminal).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentCreation {
    /// The request id the daemon gave.
    pub request_id: String,
    /// The hash the phone must approve (the daemon checks it).
    pub request_hash: String,
    /// The state.
    pub state: AgentCreationState,
    /// The agent's anchor, once the daemon names it.
    pub agent_anchor: Option<String>,
    /// When the request expires (unix seconds), while pending.
    pub expires_at: Option<u64>,
}

/// The PRX runtime state on this computer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentHostState {
    /// Not installed.
    Absent,
    /// Being installed.
    Installing,
    /// Running.
    Running,
    /// Installed and stopped.
    Stopped,
    /// Running an older runtime than the daemon: restart it.
    Stale,
    /// Not known yet.
    Unknown,
}

/// The runtime status shown in the agents settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentHostStatus {
    /// The state.
    pub state: AgentHostState,
    /// The installed PRX version, when known.
    pub prx_version: Option<String>,
    /// The refusal of the last runtime action, if it failed.
    pub refusal: Option<AgentRefusal>,
    /// The state directory of the runtime (its `runtime.json` lives there),
    /// when the host named one. The owned daemon adopts it on restart.
    #[serde(default)]
    pub state_dir: Option<PathBuf>,
    /// Whether this status answers an install or a restart that has just
    /// been applied: the owned daemon must restart to adopt the runtime.
    #[serde(default)]
    pub applied: bool,
    /// The number of the runtime action this status answers. Every action
    /// the owner submits takes the next number, so every answer differs
    /// from the one before it, even when the runtime did not change.
    #[serde(default)]
    pub nonce: u64,
}

impl AgentHostStatus {
    /// Not known yet: the status is read before anything is shown.
    pub fn unknown() -> Self {
        Self {
            state: AgentHostState::Unknown,
            prx_version: None,
            refusal: None,
            state_dir: None,
            applied: false,
            nonce: 0,
        }
    }

    /// The state directory the owned daemon restarts with, when this is the
    /// answer to the runtime action submitted as `nonce`, it installed or
    /// restarted a running runtime, and it was not refused. Any other
    /// answer (an earlier one, or one for another action) adopts nothing.
    pub fn adoption_for(&self, nonce: u64) -> Option<&std::path::Path> {
        let fresh = self.nonce == nonce && self.applied && self.refusal.is_none();
        if !fresh || self.state != AgentHostState::Running {
            return None;
        }
        self.state_dir.as_deref()
    }
}

/// What the panel's follow step does after looking at a runtime action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostStep {
    /// The action is not over: its answer has not arrived, or its
    /// confirmation is still open.
    Waiting,
    /// The action is over. When `adopt` names a state directory, the owned
    /// daemon restarts with it so it adopts the runtime.
    Ended {
        /// The state directory the owned daemon restarts with, if any.
        adopt: Option<PathBuf>,
    },
}

/// A runtime action the panel submitted and has not seen end. This is the
/// whole rule of the follow step: the action ends only with a status that
/// carries its own number (an answer, or the refusal the bridge reports for
/// a failed action), or when the owner declines its confirmation. Failures
/// of other requests running at the same time never end it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostAction {
    nonce: u64,
    install: bool,
    confirming: bool,
}

impl HostAction {
    /// An action submitted with `nonce`; `install` when it installs.
    pub fn new(nonce: u64, install: bool) -> Self {
        Self { nonce, install, confirming: false }
    }

    /// The number the action was submitted with.
    pub fn nonce(&self) -> u64 {
        self.nonce
    }

    /// The action waits for the owner's confirmation: it is not running yet.
    pub fn await_confirmation(&mut self) {
        self.confirming = true;
    }

    /// Whether an install is running: submitted, confirmed, not ended.
    pub fn installing(&self) -> bool {
        self.install && !self.confirming
    }

    /// The confirmation strip closed. A decline ends the action when it was
    /// the one waiting for it; a confirmation lets it run (its answer ends
    /// it).
    pub fn confirmation_closed(&mut self, declined: bool) -> HostStep {
        let was_confirming = std::mem::take(&mut self.confirming);
        if declined && was_confirming { HostStep::Ended { adopt: None } } else { HostStep::Waiting }
    }

    /// Looks at the stored status: only the status carrying this action's
    /// number ends it, and only an applied, unrefused, running one adopts.
    pub fn follow(&self, host: Option<&AgentHostStatus>) -> HostStep {
        match host.filter(|host| host.nonce == self.nonce) {
            Some(host) => HostStep::Ended {
                adopt: host.adoption_for(self.nonce).map(std::path::Path::to_path_buf),
            },
            None => HostStep::Waiting,
        }
    }
}

/// A runtime action the owner asks for. `Update` is not one: its manifest
/// is unavailable (see [`AgentUnavailable::RuntimeUpdate`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentRuntimeAction {
    /// Install PRX with the chosen provider and model.
    Install,
    /// Restart the runtime (and the daemon that adopts it).
    Restart,
    /// Stop the runtime.
    Stop,
    /// Uninstall the runtime.
    Uninstall,
    /// Read the status.
    Status,
}

/// What to do with one PRX job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentJobAction {
    /// Pause it.
    Pause,
    /// Resume it.
    Resume,
    /// Remove it.
    Remove,
}

/// The tools an agent may use: every tool the runtime offers, or none.
/// Per-tool selection is not offered yet (see [`AgentUnavailable::PerToolApproval`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentTools {
    /// Every tool (`["*"]` on the bus).
    All,
    /// No tool (`[]` on the bus).
    None,
}

impl AgentTools {
    /// The bus list for this choice.
    pub fn bus_list(self) -> Vec<&'static str> {
        match self {
            Self::All => vec!["*"],
            Self::None => Vec::new(),
        }
    }
}

/// The runtime configuration of a new agent. Limits are always sent: the
/// daemon publishes the runtime agent only when they are present.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentConfig {
    /// The provider id (free text until the PRX catalog exists).
    pub provider: String,
    /// The model id (free text until the PRX catalog exists).
    pub model: String,
    /// The autonomy token. The daemon publishes `Full` whatever is sent.
    pub autonomy: String,
    /// The tools.
    pub tools: AgentTools,
    /// An optional system prompt; PRX does not apply it yet.
    pub prompt: Option<String>,
    /// Concurrent turns allowed.
    pub max_concurrent_turns: u32,
    /// Cost allowed per day, in cents.
    pub max_cost_per_day_cents: u64,
}

/// A request to create a named agent, as the wizard collects it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentCreateRequest {
    /// The persona.
    pub persona: AgentPersona,
    /// The runtime configuration.
    pub config: AgentConfig,
    /// Who may reach the agent.
    pub access: AgentAccess,
}

/// The kind of a PRX job (a scheduled or a task job).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentJobKind {
    /// A cron job.
    Cron,
    /// A xin task.
    XinTask,
    /// A xin goal.
    XinGoal,
}

impl AgentJobKind {
    /// The kind the bus uses.
    pub fn code(self) -> &'static str {
        match self {
            Self::Cron => "cron",
            Self::XinTask => "xin_task",
            Self::XinGoal => "xin_goal",
        }
    }
}

/// A PRX job of an agent, as the runtime lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentJob {
    /// The job's id.
    pub id: String,
    /// Its kind.
    pub kind: AgentJobKind,
    /// The agent that created it, when the runtime names one.
    pub creator_agent: Option<String>,
}

/// A pending or decided approval of an agent in a conversation (D40).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentApproval {
    /// The approval id.
    pub approval_id: String,
    /// The conversation it belongs to.
    pub conversation_id: String,
    /// The agent the runtime names (an anchor or a runtime name; empty when
    /// the record carries none). Match it with [`AgentRecord::is_named`].
    pub agent: String,
    /// The tool the agent wants to use. Supplied by the agent: plain text,
    /// never trusted.
    pub tool: String,
    /// What the agent says it will do. Supplied by the agent: plain text,
    /// never trusted.
    pub preview: String,
    /// The status the daemon reports.
    pub status: String,
    /// When it expires (unix seconds), read from the RFC 3339 text the core
    /// stores. `None` when the record has no expiry; an expiry that cannot
    /// be read is `Some(0)`, that is, already expired.
    pub expires_at: Option<i64>,
}

impl AgentApproval {
    /// Whether the approval has expired at `now_ms` (unix milliseconds).
    #[must_use]
    pub fn is_expired(&self, now_ms: i64) -> bool {
        self.expires_at.is_some_and(|at| at.saturating_mul(1000) <= now_ms)
    }
}

/// Why an account cannot be the owner of an agent: the daemon accepts only
/// an unlocked, connected person's account that exists on that daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentCustodyProblem {
    /// The daemon does not list the account.
    Unknown,
    /// The account is locked: the owner must unlock it first.
    Locked,
    /// The account is not a person's.
    NotPerson,
    /// The account is not connected to its node yet.
    NotConnected,
}

/// Where the roster is: loaded, loading, offline or failed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentRosterState {
    /// Not read yet.
    Loading,
    /// Read; the records are current.
    Ready,
    /// No network: the records shown are the last read.
    Offline,
    /// The read failed, with the reason.
    Failed(AgentRefusal),
}

/// A feature this desktop shows as unavailable, with its owner. The reason
/// is shown next to a disabled control; nothing is sent for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentUnavailable {
    /// Sub-agents: the main/sub relation does not exist in core or PRX yet.
    SubAgent,
    /// Skills per agent (core field, then PRX).
    Skills,
    /// Autonomy is applied by PRX only at its default (core publish).
    AutonomyHonoured,
    /// Per-tool approval mode (core and PRX).
    PerToolApproval,
    /// Binding an agent to a runtime host (`host_not_supported`, core).
    HostBinding,
    /// An avatar image upload (core); the preset avatar is used.
    AvatarUpload,
    /// A provider key per agent (PRX and core); agents use the runtime's key.
    PerAgentKey,
    /// Retiring an agent: the primary's action only.
    Retire,
    /// Signed PRX updates (RFGUI-79).
    RuntimeUpdate,
    /// Creating an agent from a phone (not offered by design).
    CreateOnPhone,
}

impl AgentUnavailable {
    /// The message key of the reason (English copy lives in the locales).
    pub fn key(self) -> &'static str {
        match self {
            Self::SubAgent => "agents.unavailable.sub_agent",
            Self::Skills => "agents.unavailable.skills",
            Self::AutonomyHonoured => "agents.unavailable.autonomy",
            Self::PerToolApproval => "agents.unavailable.per_tool_approval",
            Self::HostBinding => "agents.unavailable.host_binding",
            Self::AvatarUpload => "agents.unavailable.avatar_upload",
            Self::PerAgentKey => "agents.unavailable.per_agent_key",
            Self::Retire => "agents.unavailable.retire",
            Self::RuntimeUpdate => "agents.unavailable.runtime_update",
            Self::CreateOnPhone => "agents.unavailable.create_on_phone",
        }
    }
}
