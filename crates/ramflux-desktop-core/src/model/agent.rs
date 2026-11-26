// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Named agents of an owner (AGENT-IDENTITY-01), as the desktop sees them.
//!
//! The daemon owns the records: the desktop keeps the last record it was
//! given, the state of the creation it started, the runtime (PRX) status
//! and the jobs and approvals of the runtime. Nothing here holds a provider
//! key, a credential or a passphrase. Everything the core or PRX cannot do
//! yet is a typed [`AgentUnavailable`] reason, never a silent default.

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
    /// The operation is not offered for this record (for example, delete
    /// of a retired agent).
    Unsupported,
    /// The daemon runs without PRX.
    PrxUnconfigured,
    /// The daemon does not take a host binding.
    HostNotSupported,
    /// PRX is not installed on this computer.
    PrxMissing,
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
    /// The daemon or PRX did not answer in time.
    Timeout,
    /// The connection to the daemon broke.
    Transport,
    /// A code this desktop has no wording for.
    Other(String),
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
}

impl AgentHostStatus {
    /// Not known yet: the status is read before anything is shown.
    pub fn unknown() -> Self {
        Self { state: AgentHostState::Unknown, prx_version: None, refusal: None }
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
    /// The status the daemon reports.
    pub status: String,
    /// When it expires (unix seconds).
    pub expires_at: Option<u64>,
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
    /// Editing the configuration after creation (no bus method, core).
    ConfigEdit,
    /// Autonomy is applied by PRX only at its default (core publish).
    AutonomyHonoured,
    /// The PRX catalog of models and tools (no `agent.catalog`, core).
    Catalog,
    /// Per-tool approval mode (core and PRX).
    PerToolApproval,
    /// Agent-level pause or enable (core).
    AgentPause,
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
            Self::ConfigEdit => "agents.unavailable.config_edit",
            Self::AutonomyHonoured => "agents.unavailable.autonomy",
            Self::Catalog => "agents.unavailable.catalog",
            Self::PerToolApproval => "agents.unavailable.per_tool_approval",
            Self::AgentPause => "agents.unavailable.agent_pause",
            Self::HostBinding => "agents.unavailable.host_binding",
            Self::AvatarUpload => "agents.unavailable.avatar_upload",
            Self::PerAgentKey => "agents.unavailable.per_agent_key",
            Self::Retire => "agents.unavailable.retire",
            Self::RuntimeUpdate => "agents.unavailable.runtime_update",
            Self::CreateOnPhone => "agents.unavailable.create_on_phone",
        }
    }
}
