// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Named agents (AGENT-IDENTITY-01): the daemon's answers as events, the
//! typed refusal of each failed request, and the PRX runtime status.
//!
//! The daemon answers ProtoJSON, whose field names are lower camel case
//! (`agentAnchor`), and it writes 64-bit numbers as strings; both spellings
//! are read here, and a missing field stays missing rather than invented.
//! A refusal reaches the desktop as the daemon's `reason_code` (or its
//! `code`) and becomes an [`AgentRefusal`]; the failed effect's message is
//! that refusal's locale key, never the daemon's English.

use ramflux_desktop_core::{
    AgentAccess, AgentAccessMode, AgentApproval, AgentCatalog, AgentCreation, AgentCreationState,
    AgentHostState, AgentHostStatus, AgentJob, AgentJobKind, AgentLifecycle, AgentPersona,
    AgentRecord, AgentRefusal, AgentSettings, CatalogModel, CatalogTool, DomainEvent, methods,
};
use serde_json::Value;

use crate::bus::BusError;
use crate::plan::agents::{RUNTIME_METHOD, prx_name};
use crate::plan::fetch::Fetch;
use crate::prx_host;
use crate::translate::Translated;

/// The first non-null value under one of `keys`.
fn pick<'a>(value: &'a Value, keys: &[&str]) -> &'a Value {
    keys.iter().map(|key| &value[*key]).find(|found| !found.is_null()).unwrap_or(&Value::Null)
}

/// The first non-empty string under one of `keys`, or empty.
fn text_of(value: &Value, keys: &[&str]) -> String {
    pick(value, keys).as_str().unwrap_or_default().to_owned()
}

/// A 64-bit number, written as a number or as a string.
fn number_of(value: &Value, keys: &[&str]) -> Option<u64> {
    match pick(value, keys) {
        Value::Number(number) => number.as_u64(),
        Value::String(text) => text.parse().ok(),
        _ => None,
    }
}

/// The refusal a daemon or PRX code names.
pub fn refusal_of_code(code: &str) -> AgentRefusal {
    match code {
        "confirmation_required" => AgentRefusal::ConfirmationRequired,
        "desktop_required" => AgentRefusal::DesktopRequired,
        "creation_pairing_required" => AgentRefusal::CreationPairingRequired,
        "metadata_pending" => AgentRefusal::MetadataPending,
        "access_policy_invalid" => AgentRefusal::AccessPolicyInvalid,
        "persona_invalid" => AgentRefusal::PersonaInvalid,
        "stale_base" => AgentRefusal::StaleBase,
        "revision_conflict" => AgentRefusal::RevisionConflict,
        "unsupported" => AgentRefusal::Unsupported,
        "prx_unconfigured" => AgentRefusal::PrxUnconfigured,
        "host_not_supported" => AgentRefusal::HostNotSupported,
        "prx_rejected" => AgentRefusal::PrxRejected,
        "prx_unavailable" => AgentRefusal::PrxUnavailable,
        "auth_failed" => AgentRefusal::AuthFailed,
        "prx_unsupported" => AgentRefusal::PrxUnsupported,
        "cron_job_unknown" => AgentRefusal::CronJobUnknown,
        "xin_task_unknown" => AgentRefusal::XinTaskUnknown,
        "xin_goal_unknown" => AgentRefusal::XinGoalUnknown,
        "field_invalid" => AgentRefusal::FieldInvalid,
        "agent_invalid" => AgentRefusal::AgentInvalid,
        "storage_unavailable" => AgentRefusal::StorageUnavailable,
        "prx_missing" | "runtime_missing" => AgentRefusal::PrxMissing,
        "prx_path_unsafe" => AgentRefusal::PrxPathUnsafe,
        "prx_path_changed" => AgentRefusal::PrxPathChanged,
        "provider_missing" => AgentRefusal::ProviderMissing,
        "credential_unavailable" => AgentRefusal::CredentialUnavailable,
        "runtime_stale" => AgentRefusal::RuntimeStale,
        "ancestor_rejected" => AgentRefusal::AncestorRejected,
        "manifest_unavailable" => AgentRefusal::ManifestUnavailable,
        "unsupported_platform" => AgentRefusal::UnsupportedPlatform,
        "prx_command" => AgentRefusal::PrxCommand,
        "prx_spawn" => AgentRefusal::PrxSpawn,
        other => AgentRefusal::Other(other.to_owned()),
    }
}

/// The refusal of a failed agent request.
pub fn refusal_of(error: &BusError) -> AgentRefusal {
    match error {
        BusError::Timeout => AgentRefusal::Timeout,
        BusError::Transport(_) => AgentRefusal::Transport,
        BusError::Daemon { code, reason_code, .. } => {
            refusal_of_code(reason_code.as_deref().filter(|r| !r.is_empty()).unwrap_or(code))
        }
    }
}

/// Whether the daemon refused a watch because it does not know the cursor
/// (above its stored head): the caller lists again.
pub fn is_stale_cursor(method: &str, error: &BusError) -> bool {
    method == methods::AGENT_WATCH && refusal_code(error) == Some("journal_cursor_invalid")
}

fn refusal_code(error: &BusError) -> Option<&str> {
    match error {
        BusError::Daemon { code, reason_code, .. } => {
            Some(reason_code.as_deref().filter(|r| !r.is_empty()).unwrap_or(code))
        }
        _ => None,
    }
}

/// The status a failed runtime action reports: the typed refusal and the
/// number of the action, so the panel that submitted it sees it end.
pub fn host_refusal(error: &BusError, nonce: u64) -> AgentHostStatus {
    AgentHostStatus {
        state: AgentHostState::Unknown,
        prx_version: None,
        refusal: Some(refusal_of(error)),
        state_dir: None,
        applied: false,
        nonce,
    }
}

/// Whether a refusal came from the PRX runtime (the daemon's `prx_*` codes
/// and the runtime states they name).
fn runtime_refusal(refusal: &AgentRefusal) -> bool {
    matches!(
        refusal,
        AgentRefusal::PrxUnconfigured
            | AgentRefusal::PrxMissing
            | AgentRefusal::PrxPathUnsafe
            | AgentRefusal::PrxPathChanged
            | AgentRefusal::ProviderMissing
            | AgentRefusal::CredentialUnavailable
            | AgentRefusal::RuntimeStale
            | AgentRefusal::AncestorRejected
            | AgentRefusal::ManifestUnavailable
            | AgentRefusal::UnsupportedPlatform
            | AgentRefusal::PrxCommand
            | AgentRefusal::PrxSpawn
            | AgentRefusal::PrxRejected
            | AgentRefusal::PrxUnavailable
            | AgentRefusal::AuthFailed
            | AgentRefusal::PrxUnsupported
            | AgentRefusal::FieldInvalid
            | AgentRefusal::AgentInvalid
            | AgentRefusal::StorageUnavailable
    )
}

/// The locale keys whose hint is a restart of the runtime (see
/// [`AgentRefusal::offers_restart`]).
pub const RESTART_HINT_KEYS: [&str; 2] =
    ["backend.agent.prx.auth_failed", "backend.agent.prx.unavailable"];

/// The locale key of a refusal. Every [`AgentRefusal`] has one.
pub fn refusal_key(refusal: &AgentRefusal) -> &'static str {
    match refusal {
        AgentRefusal::ConfirmationRequired => "backend.agent.confirmation_required",
        AgentRefusal::DesktopRequired => "backend.agent.desktop_required",
        AgentRefusal::CreationPairingRequired => "backend.agent.creation_pairing_required",
        AgentRefusal::MetadataPending => "backend.agent.metadata_pending",
        AgentRefusal::AccessPolicyInvalid => "backend.agent.access_policy_invalid",
        AgentRefusal::PersonaInvalid => "backend.agent.persona_invalid",
        AgentRefusal::StaleBase => "backend.agent.stale_base",
        AgentRefusal::RevisionConflict => "backend.agent.revision_conflict",
        AgentRefusal::Unsupported => "backend.agent.unsupported",
        AgentRefusal::PrxUnconfigured => "backend.agent.prx.unconfigured",
        AgentRefusal::HostNotSupported => "backend.agent.host_not_supported",
        AgentRefusal::PrxMissing => "backend.agent.prx.missing",
        AgentRefusal::PrxPathUnsafe => "backend.agent.prx.path_unsafe",
        AgentRefusal::PrxPathChanged => "backend.agent.prx.path_changed",
        AgentRefusal::ProviderMissing => "backend.agent.prx.provider_missing",
        AgentRefusal::CredentialUnavailable => "backend.agent.prx.credential_unavailable",
        AgentRefusal::RuntimeStale => "backend.agent.prx.runtime_stale",
        AgentRefusal::AncestorRejected => "backend.agent.prx.ancestor_rejected",
        AgentRefusal::ManifestUnavailable => "backend.agent.prx.manifest_unavailable",
        AgentRefusal::UnsupportedPlatform => "backend.agent.prx.unsupported_platform",
        AgentRefusal::PrxCommand => "backend.agent.prx.command",
        AgentRefusal::PrxSpawn => "backend.agent.prx.spawn",
        AgentRefusal::PrxRejected => "backend.agent.prx.rejected",
        AgentRefusal::PrxUnavailable => "backend.agent.prx.unavailable",
        AgentRefusal::AuthFailed => "backend.agent.prx.auth_failed",
        AgentRefusal::PrxUnsupported => "backend.agent.prx.unsupported",
        AgentRefusal::CronJobUnknown => "backend.agent.job.cron_unknown",
        AgentRefusal::XinTaskUnknown => "backend.agent.job.xin_task_unknown",
        AgentRefusal::XinGoalUnknown => "backend.agent.job.xin_goal_unknown",
        AgentRefusal::FieldInvalid => "backend.agent.prx.field_invalid",
        AgentRefusal::AgentInvalid => "backend.agent.prx.agent_invalid",
        AgentRefusal::StorageUnavailable => "backend.agent.prx.storage_unavailable",
        AgentRefusal::Timeout => "backend.agent.timeout",
        AgentRefusal::Transport => "backend.agent.transport",
        AgentRefusal::Other(_) => "backend.agent.other",
    }
}

/// The message of a failed agent effect: the refusal's key. A creation
/// that failed on the runtime is shown as created with its runtime settings
/// not applied, since the daemon stages the agent before it publishes it.
pub fn failure_key(method: &str, error: &BusError) -> Option<&'static str> {
    if !crate::plan::agents::is_agent_method(method) && method != RUNTIME_METHOD {
        return None;
    }
    let refusal = refusal_of(error);
    if method == methods::AGENT_CREATE && runtime_refusal(&refusal) {
        return Some("backend.agent.create.runtime_not_applied");
    }
    Some(refusal_key(&refusal))
}

/// The creation request as the daemon's custody status describes it.
pub fn creation_of(custody: &Value) -> Option<AgentCreation> {
    let request_id = text_of(custody, &["requestId", "request_id"]);
    if request_id.is_empty() {
        return None;
    }
    let refused = pick(custody, &["error"]);
    let state = if refused.is_object() {
        AgentCreationState::Failed(refusal_of_code(&text_of(
            refused,
            &["reasonCode", "reason_code"],
        )))
    } else {
        AgentCreationState::from_custody_state(&text_of(custody, &["state"]))
    };
    let agent_anchor = Some(text_of(custody, &["agentAnchor", "agent_anchor"]))
        .filter(|anchor| !anchor.is_empty());
    Some(AgentCreation {
        request_id,
        // The phone checks its own hash of the plan; this desktop shows none.
        request_hash: String::new(),
        state,
        agent_anchor,
        expires_at: number_of(custody, &["expiresAt", "expires_at"]),
    })
}

fn mode_of(token: &str) -> AgentAccessMode {
    match token {
        "contacts" => AgentAccessMode::Contacts,
        "allowlist" => AgentAccessMode::Allowlist,
        "open" => AgentAccessMode::Open,
        // Owner only, and an unknown token too: the most restrictive reading.
        _ => AgentAccessMode::Owner,
    }
}

fn access_of(value: &Value) -> AgentAccess {
    let allowed = pick(value, &["allowedHumans", "allowed_humans"])
        .as_array()
        .map(|list| list.iter().filter_map(|a| a.as_str().map(str::to_owned)).collect())
        .unwrap_or_default();
    AgentAccess {
        dm: mode_of(&text_of(value, &["dm"])),
        group_inviters: mode_of(&text_of(value, &["groupInviters", "group_inviters"])),
        allowed_humans: allowed,
    }
}

/// An agent record as the daemon reports it. A record without an anchor is
/// not one this desktop can show.
pub fn record_of(value: &Value) -> Option<AgentRecord> {
    let agent_anchor = text_of(value, &["agentAnchor", "agent_anchor"]);
    if agent_anchor.is_empty() {
        return None;
    }
    let persona = pick(value, &["persona"]);
    let runtime_name = prx_name(&agent_anchor).unwrap_or_default();
    Some(AgentRecord {
        agent_anchor,
        custodian_anchor: text_of(value, &["custodianAnchor", "custodian_anchor"]),
        persona: AgentPersona {
            name: strip_format(&text_of(persona, &["name"])),
            avatar: text_of(persona, &["avatar"]),
            responsibility: strip_format(&text_of(persona, &["responsibility"])),
        },
        lifecycle: AgentLifecycle::from_code(&text_of(value, &["lifecycle"])),
        access: access_of(pick(value, &["access"])),
        role: text_of(value, &["role"]),
        base: pick(value, &["base"]).clone(),
        runtime_name,
        settings: settings_of(value),
    })
}

fn job_of(value: &Value) -> Option<AgentJob> {
    let id = text_of(value, &["id"]);
    let kind = match text_of(value, &["kind"]).as_str() {
        "cron" => AgentJobKind::Cron,
        "xin_task" => AgentJobKind::XinTask,
        "xin_goal" => AgentJobKind::XinGoal,
        _ => return None,
    };
    if id.is_empty() {
        return None;
    }
    let creator = strip_format(&text_of(value, &["creatorAgent", "creator_agent"]));
    Some(AgentJob { id, kind, creator_agent: Some(creator).filter(|creator| !creator.is_empty()) })
}

fn approval_of(value: &Value) -> Option<AgentApproval> {
    let approval_id = text_of(value, &["approvalId", "approval_id"]);
    if approval_id.is_empty() {
        return None;
    }
    Some(AgentApproval {
        approval_id,
        conversation_id: text_of(value, &["conversationId", "conversation_id"]),
        agent: text_of(value, &["agent"]),
        tool: plain_line(&text_of(value, &["tool"]), 128),
        preview: plain_line(&text_of(value, &["preview"]), 512),
        status: approval_status(value),
        expires_at: expiry_of(&text_of(value, &["expiresAt", "expires_at"])),
    })
}

/// The state of an approval the daemon answered. An answered approval is
/// stored as `decided` with its `decision`; the desktop reads that as
/// `approved` or `denied`. Any other pair is kept as the daemon sent it.
fn approval_status(value: &Value) -> String {
    let status = text_of(value, &["status"]);
    if status != "decided" {
        return status;
    }
    match text_of(value, &["decision"]).as_str() {
        decision @ ("approved" | "denied") => decision.to_owned(),
        _ => status,
    }
}

/// The unix seconds of an RFC 3339 expiry the core stored as text. Empty
/// text is no expiry; text that is not RFC 3339 reads as already expired, so
/// an approval whose expiry cannot be told is never offered.
fn expiry_of(text: &str) -> Option<i64> {
    if text.is_empty() {
        return None;
    }
    Some(chrono::DateTime::parse_from_rfc3339(text).map_or(0, |at| at.timestamp()))
}

/// `text` without any format character (General_Category `Cf`): the
/// bidirectional controls and isolates, zero-width characters, the word
/// joiner, the soft hyphen, the byte order mark, tag characters and the rest
/// of the category. Every agent text shown passes through here, so the text
/// cannot reorder or hide what is drawn around it.
pub fn strip_format(text: &str) -> String {
    use unicode_general_category::{GeneralCategory, get_general_category};
    text.chars().filter(|c| get_general_category(*c) != GeneralCategory::Format).collect()
}

/// Text an agent supplied, made safe to show as one plain line: control
/// characters become spaces, every format character is removed, runs of white
/// space collapse, and at most `max` characters are kept.
fn plain_line(raw: &str, max: usize) -> String {
    let spaced: String =
        strip_format(raw).chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    spaced.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(max).collect()
}

/// The runtime status the host answered, without any secret. `action` is the
/// runtime action the answer is for (its name, as the effect body carries
/// it): an install or a restart marks the status as applied. `nonce` is the
/// number the action was submitted with.
pub fn host_status_of(answer: &Value, action: &str, nonce: u64) -> AgentHostStatus {
    let state = match answer["state"].as_str().unwrap_or_default() {
        "absent" => AgentHostState::Absent,
        "running" => AgentHostState::Running,
        "stopped" => AgentHostState::Stopped,
        "stale" => AgentHostState::Stale,
        "installing" => AgentHostState::Installing,
        _ => AgentHostState::Unknown,
    };
    let version = answer["prx_version"].as_str().unwrap_or_default();
    AgentHostStatus {
        state,
        prx_version: Some(version.to_owned()).filter(|v| !v.is_empty()),
        refusal: None,
        state_dir: prx_host::state_dir_of(answer),
        applied: matches!(action, "install" | "restart"),
        nonce,
    }
}

/// The PRX catalog an `agent.catalog` answer holds.
fn catalog_of(value: &Value) -> AgentCatalog {
    let rows = |key: &str| value[key].as_array().cloned().unwrap_or_default();
    AgentCatalog {
        prx_version: text_of(value, &["prx_version", "prxVersion"]),
        revision: text_of(value, &["revision"]),
        models: rows("models")
            .iter()
            .filter_map(|row| {
                let (provider, model) = (text_of(row, &["provider"]), text_of(row, &["model"]));
                (!provider.is_empty() && !model.is_empty()).then(|| CatalogModel {
                    provider,
                    model,
                    credentials_ready: pick(row, &["credentials_ready", "credentialsReady"])
                        .as_bool()
                        .unwrap_or(false),
                })
            })
            .collect(),
        tools: rows("tools")
            .iter()
            .filter_map(|row| {
                let name = text_of(row, &["name"]);
                (!name.is_empty()).then(|| CatalogTool {
                    name,
                    read_only: pick(row, &["read_only", "readOnly"]).as_bool().unwrap_or(false),
                })
            })
            .collect(),
        default_autonomy: text_of(pick(value, &["defaults"]), &["autonomy"]),
        reserved_names: rows("reserved_names")
            .iter()
            .chain(rows("reservedNames").iter())
            .filter_map(|name| name.as_str().map(str::to_owned))
            .collect(),
    }
}

/// The configuration a record (or an update's answer) shows: the `config`
/// object and the per-agent `revision` beside it; `None` without a config.
fn settings_of(record: &Value) -> Option<AgentSettings> {
    let config = pick(record, &["config"]);
    if !config.is_object() {
        return None;
    }
    let limits = pick(config, &["limits"]);
    Some(AgentSettings {
        provider: text_of(config, &["provider"]),
        model: text_of(config, &["model"]),
        autonomy: text_of(config, &["autonomy"]),
        prompt: Some(text_of(config, &["prompt"])).filter(|text| !text.is_empty()),
        tools: pick(config, &["tools"])
            .as_array()
            .map(|names| names.iter().filter_map(|n| n.as_str().map(str::to_owned)).collect())
            .unwrap_or_default(),
        enabled: pick(config, &["enabled"]).as_bool().unwrap_or(true),
        max_concurrent_turns: number_of(limits, &["max_concurrent_turns", "maxConcurrentTurns"])
            .and_then(|turns| u32::try_from(turns).ok()),
        max_cost_per_day_cents: number_of(
            limits,
            &["max_cost_per_day_cents", "maxCostPerDayCents"],
        ),
        revision: Some(text_of(record, &["revision"])).filter(|text| !text.is_empty()),
    })
}

/// The events of a performed agent request.
pub fn effect_events(method: &str, answer: &Value) -> Vec<DomainEvent> {
    match method {
        methods::AGENT_CREATE | methods::AGENT_OPERATION_GET => {
            creation_of(pick(answer, &["custody"]))
                .map(DomainEvent::AgentCreationUpdated)
                .into_iter()
                .collect()
        }
        methods::AGENT_CATALOG => vec![DomainEvent::AgentCatalogLoaded(catalog_of(answer))],
        // The record of one agent: its configuration and its revision.
        methods::AGENT_GET => {
            let record = pick(answer, &["record"]);
            record_of(record)
                .map(|record| DomainEvent::AgentRecordUpdated(Box::new(record)))
                .into_iter()
                .collect()
        }
        // The answer names the agent, its revision and its configuration now.
        methods::AGENT_UPDATE | methods::AGENT_PAUSE | methods::AGENT_RESUME => {
            let anchor = text_of(answer, &["agent_anchor"]);
            settings_of(answer)
                .filter(|_| !anchor.is_empty())
                .map(|settings| DomainEvent::AgentSettingsChanged {
                    agent_anchor: anchor,
                    settings,
                })
                .into_iter()
                .collect()
        }
        methods::AGENT_PERSONA_EDIT | methods::AGENT_ACCESS_SET | methods::AGENT_DELETE => {
            record_of(pick(answer, &["record"]))
                .map(|record| DomainEvent::AgentRecordUpdated(Box::new(record)))
                .into_iter()
                .collect()
        }
        _ => Vec::new(),
    }
}

/// The events of a read of agent state.
pub fn read_events(fetch: &Fetch, value: &Value) -> Vec<DomainEvent> {
    match fetch {
        Fetch::AgentList | Fetch::AgentListPage(_) => {
            let page = pick(value, &["page"]);
            let records = page["records"]
                .as_array()
                .map(|list| list.iter().filter_map(record_of).collect())
                .unwrap_or_default();
            // Only the first page of a listing (and a watch answer) names
            // the cursor: a later page carries the head as of its own read,
            // which could skip a change made between the pages.
            let journal_cursor = match fetch {
                Fetch::AgentListPage(_) => String::new(),
                _ => text_of(page, &["journalCursor", "journal_cursor"]),
            };
            vec![DomainEvent::AgentsListed { records, journal_cursor }]
        }
        Fetch::AgentApprovals => {
            let approvals = value["approvals"]
                .as_array()
                .map(|list| list.iter().filter_map(approval_of).collect())
                .unwrap_or_default();
            vec![DomainEvent::AgentApprovalsUpdated(approvals)]
        }
        Fetch::AgentJobs(anchor) => {
            let jobs = value["jobs"]
                .as_array()
                .map(|list| list.iter().filter_map(job_of).collect())
                .unwrap_or_default();
            vec![DomainEvent::AgentJobsUpdated { agent: anchor.clone(), jobs }]
        }
        Fetch::AgentCatalog => vec![DomainEvent::AgentCatalogLoaded(catalog_of(value))],
        Fetch::AgentGet(anchor) => settings_of(pick(value, &["record"]))
            .map(|settings| DomainEvent::AgentSettingsChanged {
                agent_anchor: anchor.clone(),
                settings,
            })
            .into_iter()
            .collect(),
        Fetch::AgentOperation(_) => creation_of(pick(value, &["custody"]))
            .map(DomainEvent::AgentCreationUpdated)
            .into_iter()
            .collect(),
        _ => Vec::new(),
    }
}

/// A page of the roster: its records, and the read of its `next` page while
/// the daemon has one.
pub fn list_page(fetch: &Fetch, value: &Value) -> Translated {
    let next = text_of(pick(value, &["page"]), &["next"]);
    let follow = if next.is_empty() { Vec::new() } else { vec![Fetch::AgentListPage(next)] };
    Translated { events: read_events(fetch, value), follow }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_custody_status_maps_each_daemon_state() {
        let pending = json!({"custody": {"requestId": "r-1", "state": "authorization_pending",
            "expiresAt": "1900000000"}});
        let creation = creation_of(&pending["custody"]).expect("a request id");
        assert_eq!(creation.state, AgentCreationState::PendingPhone);
        assert_eq!(creation.expires_at, Some(1_900_000_000));
        let done = json!({"requestId": "r-1", "state": "complete", "agentAnchor": "anc"});
        assert_eq!(creation_of(&done).map(|c| c.state), Some(AgentCreationState::Created));
        let refused = json!({"requestId": "r-1", "state": "refused",
            "error": {"reasonCode": "prx_spawn"}});
        assert_eq!(
            creation_of(&refused).map(|c| c.state),
            Some(AgentCreationState::Failed(AgentRefusal::PrxSpawn))
        );
    }

    #[test]
    fn a_record_reads_both_spellings_and_keeps_its_base() {
        let camel = json!({
            "agentAnchor": "anc", "custodianAnchor": "owner", "lifecycle": "U", "role": "named",
            "persona": {"name": "N", "avatar": "preset:spark:teal", "responsibility": "R"},
            "access": {"dm": "contacts", "groupInviters": "owner", "allowedHumans": []},
            "base": {"id": "b"},
        });
        let record = record_of(&camel).expect("an anchor");
        assert_eq!(record.lifecycle, AgentLifecycle::Active);
        assert_eq!(record.access.dm, AgentAccessMode::Contacts);
        assert_eq!(record.base, json!({"id": "b"}));
        let snake = json!({"agent_anchor": "anc", "lifecycle": "X"});
        assert_eq!(record_of(&snake).map(|r| r.lifecycle), Some(AgentLifecycle::Retired));
        assert!(record_of(&json!({"lifecycle": "U"})).is_none());
    }

    #[test]
    fn an_unknown_access_token_is_the_owner_only() {
        let value = json!({"dm": "everyone", "group_inviters": "open"});
        let access = access_of(&value);
        assert_eq!(access.dm, AgentAccessMode::Owner);
        assert_eq!(access.group_inviters, AgentAccessMode::Open);
    }

    #[test]
    fn a_refusal_keeps_its_typed_code_and_has_a_key() {
        let error = BusError::Daemon {
            retry_after_ms: None,
            code: "ValidationFailed".into(),
            message: "english".into(),
            reason_code: Some("stale_base".into()),
            details: None,
            cause: None,
        };
        assert_eq!(refusal_of(&error), AgentRefusal::StaleBase);
        assert_eq!(refusal_key(&refusal_of(&error)), "backend.agent.stale_base");
        assert_eq!(refusal_key(&AgentRefusal::Other("x".into())), "backend.agent.other");
    }

    #[test]
    fn a_runtime_failure_of_a_creation_says_it_was_created_without_runtime() {
        let error = BusError::Daemon {
            retry_after_ms: None,
            code: "prx_failed".into(),
            message: String::new(),
            reason_code: Some("prx_unconfigured".into()),
            details: None,
            cause: None,
        };
        assert_eq!(
            failure_key(methods::AGENT_CREATE, &error),
            Some("backend.agent.create.runtime_not_applied")
        );
        assert_eq!(failure_key("contact.block", &error), None);
    }

    #[test]
    fn the_runtime_status_names_its_state_without_secrets() {
        let status =
            host_status_of(&json!({"state": "running", "prx_version": "0.1.0"}), "status", 1);
        assert_eq!(status.state, AgentHostState::Running);
        assert_eq!(status.prx_version.as_deref(), Some("0.1.0"));
        assert!(!status.applied, "a status read applies nothing");
        assert_eq!(
            host_status_of(&json!({"state": "odd"}), "status", 2).state,
            AgentHostState::Unknown
        );
    }

    #[test]
    fn an_applied_runtime_names_the_state_directory_the_daemon_adopts() {
        let answer = json!({
            "state": "running",
            "runtime_path": "/state/prx/runtime.json",
            "api_base": "http://127.0.0.1:9/",
        });
        let status = host_status_of(&answer, "restart", 3);
        assert_eq!(status.nonce, 3, "the status carries the action's number");
        assert!(status.applied);
        assert_eq!(status.state_dir, Some(std::path::PathBuf::from("/state/prx")));
        let stopped = host_status_of(&answer, "stop", 4);
        assert!(!stopped.applied);
        assert_eq!(host_status_of(&json!({"state": "absent"}), "install", 5).state_dir, None);
    }

    #[test]
    fn every_refusal_code_has_a_typed_refusal_and_a_key() {
        for (code, refusal) in [
            ("prx_rejected", AgentRefusal::PrxRejected),
            ("prx_unavailable", AgentRefusal::PrxUnavailable),
            ("auth_failed", AgentRefusal::AuthFailed),
            ("prx_unsupported", AgentRefusal::PrxUnsupported),
            ("cron_job_unknown", AgentRefusal::CronJobUnknown),
            ("xin_task_unknown", AgentRefusal::XinTaskUnknown),
            ("xin_goal_unknown", AgentRefusal::XinGoalUnknown),
            ("field_invalid", AgentRefusal::FieldInvalid),
            ("agent_invalid", AgentRefusal::AgentInvalid),
            ("storage_unavailable", AgentRefusal::StorageUnavailable),
        ] {
            assert_eq!(refusal_of_code(code), refusal, "{code}");
            assert!(!refusal_key(&refusal).ends_with(".other"), "{code} has its own wording");
        }
        assert!(AgentRefusal::AuthFailed.offers_restart());
        assert!(AgentRefusal::PrxUnavailable.offers_restart());
        assert!(!AgentRefusal::PrxRejected.offers_restart());
        let prx = BusError::Daemon {
            retry_after_ms: None,
            code: "prx_failed".into(),
            message: String::new(),
            reason_code: Some("prx_rejected".into()),
            details: None,
            cause: None,
        };
        assert_eq!(
            failure_key(methods::AGENT_CREATE, &prx),
            Some("backend.agent.create.runtime_not_applied"),
            "a PRX failure of a creation says it was created without its runtime"
        );
    }

    #[test]
    fn a_later_page_names_no_cursor_and_a_stale_cursor_is_told() {
        let page = json!({"page": {"records": [], "journal_cursor": "c-2"}});
        let events = read_events(&Fetch::AgentListPage("n-1".into()), &page);
        assert!(matches!(
            events.as_slice(),
            [DomainEvent::AgentsListed { journal_cursor, .. }] if journal_cursor.is_empty()
        ));
        let events = read_events(&Fetch::AgentList, &page);
        assert!(matches!(
            events.as_slice(),
            [DomainEvent::AgentsListed { journal_cursor, .. }] if journal_cursor == "c-2"
        ));
        let invalid = BusError::Daemon {
            retry_after_ms: None,
            code: "internal".into(),
            message: String::new(),
            reason_code: Some("journal_cursor_invalid".into()),
            details: None,
            cause: None,
        };
        assert!(is_stale_cursor(methods::AGENT_WATCH, &invalid));
        assert!(!is_stale_cursor(methods::AGENT_JOB_LIST, &invalid));
        assert!(!is_stale_cursor(methods::AGENT_WATCH, &BusError::Timeout));
    }

    #[test]
    fn an_approval_carries_its_agent_tool_preview_and_rfc3339_expiry() {
        let row = json!({
            "approval_id": "ap",
            "conversation_id": "conv-1",
            "agent": "a_name",
            "tool": "shell\u{202E}\n  run",
            "preview": "rm -rf\t/tmp/x\u{0007}",
            "status": "pending",
            "expires_at": "2030-01-01T00:00:00Z",
        });
        let approval = approval_of(&row).expect("approval");
        assert_eq!(approval.agent, "a_name");
        assert_eq!(approval.conversation_id, "conv-1");
        assert_eq!(approval.tool, "shell run", "controls and overrides never reach the screen");
        assert_eq!(approval.preview, "rm -rf /tmp/x");
        assert_eq!(approval.expires_at, Some(1_893_456_000));
        let offset = json!({"approval_id": "a", "expires_at": "2030-01-01T01:00:00+01:00"});
        assert_eq!(approval_of(&offset).expect("approval").expires_at, Some(1_893_456_000));
    }

    #[test]
    fn an_answered_approval_reads_its_decision() {
        let answered = |status: &str, decision: &str| json!({"approval_id": "ap", "status": status, "decision": decision});
        let state = |row: Value| approval_of(&row).expect("approval").status;
        assert_eq!(state(answered("decided", "approved")), "approved");
        assert_eq!(state(answered("decided", "denied")), "denied");
        assert_eq!(state(answered("decided", "maybe")), "decided", "an unknown decision is kept");
        assert_eq!(state(json!({"approval_id": "ap", "status": "pending"})), "pending");
        assert_eq!(state(answered("approved", "")), "approved");
    }

    #[test]
    fn an_expiry_that_is_missing_is_none_and_one_that_is_unreadable_is_expired() {
        assert_eq!(expiry_of(""), None);
        assert_eq!(expiry_of("tomorrow"), Some(0));
        assert_eq!(expiry_of("1893456000"), Some(0), "a number is not RFC 3339 text");
    }

    #[test]
    fn plain_line_keeps_at_most_the_limit() {
        assert_eq!(plain_line(&"x".repeat(600), 512).chars().count(), 512);
    }

    #[test]
    fn a_roster_record_knows_the_runtime_name_an_approval_uses() {
        let anchor = ramflux_protocol::encode_base64url([9u8; 32]);
        let row = json!({"agent_anchor": anchor, "lifecycle": "active"});
        let record = record_of(&row).expect("record");
        assert_eq!(record.runtime_name, prx_name(&anchor).expect("name"));
        assert!(record.is_named(&record.runtime_name));
        assert!(record.is_named(&anchor));
        assert!(!record.is_named(""));
        assert!(!record.is_named("other"));
    }

    #[test]
    fn a_roster_page_with_a_next_cursor_reads_the_next_page() {
        let page = json!({"page": {"records": [], "next": "n-1", "journal_cursor": "c-1"}});
        let more = list_page(&Fetch::AgentList, &page);
        assert_eq!(more.follow, vec![Fetch::AgentListPage("n-1".into())]);
        let last =
            list_page(&Fetch::AgentListPage("n-1".into()), &json!({"page": {"records": []}}));
        assert!(last.follow.is_empty(), "the last page ends the read");
    }

    #[test]
    fn jobs_and_approvals_read_their_rows() {
        let jobs = json!({"jobs": [
            {"id": "j-1", "kind": "xin_goal", "creator_agent": "a_x"},
            {"id": "j-2", "kind": "unknown"},
        ]});
        let events = read_events(&Fetch::AgentJobs("anc".into()), &jobs);
        let [DomainEvent::AgentJobsUpdated { agent, jobs }] = events.as_slice() else {
            panic!("test: a jobs update")
        };
        assert_eq!(agent, "anc");
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].kind, AgentJobKind::XinGoal);
        let approvals = json!({"approvals": [{"approval_id": "ap", "status": "pending"}]});
        assert!(matches!(
            read_events(&Fetch::AgentApprovals, &approvals).as_slice(),
            [DomainEvent::AgentApprovalsUpdated(list)] if list.len() == 1
        ));
    }
}
