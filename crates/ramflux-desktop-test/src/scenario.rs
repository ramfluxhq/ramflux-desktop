// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The scenario fixture format.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ramflux_desktop_core::{AccountId, DomainEvent, Effect, JsonValue, Timestamp, methods};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Default scenario epoch: 2026-10-01T00:00:00Z in unix milliseconds.
pub const DEFAULT_EPOCH: Timestamp = Timestamp(1_790_812_800_000);

/// Names of the bundled scenarios, in order. Companion fixtures (background
/// accounts of a multi-account scenario) are not listed.
pub const SCENARIO_NAMES: [&str; 12] = [
    "s01_first_launch",
    "s02_daily",
    "s03_intervention",
    "s04_safety",
    "s05_group_governance",
    "s06_in_progress",
    "s07_conflict_privacy_handoff",
    "s08_failures",
    "s09_multi_account_day",
    "s10_ai_participant",
    "s11_surface_versions",
    "s12_surface_limits",
];

/// A scripted sequence of domain events plus reactions to effects.
///
/// Offsets are milliseconds: timeline offsets from the scenario start,
/// response offsets from the moment the matching effect is submitted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scenario {
    /// Short name.
    pub name: String,
    /// What the scenario demonstrates.
    pub description: String,
    /// Wall-clock time of offset 0.
    #[serde(default = "default_epoch")]
    pub epoch: Timestamp,
    /// Events at fixed offsets.
    pub timeline: Vec<(u64, DomainEvent)>,
    /// Reactions to submitted effects.
    #[serde(default)]
    pub responses: Vec<ResponseRule>,
    /// Further accounts played beside this one as background accounts.
    #[serde(default)]
    pub companions: Vec<Companion>,
}

/// A background account played from another fixture.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Companion {
    /// Account slot the companion plays.
    pub account_id: AccountId,
    /// Fixture name, e.g. `"s09_work_account"`.
    pub scenario: String,
}

fn default_epoch() -> Timestamp {
    DEFAULT_EPOCH
}

/// Emits events when a submitted effect matches.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResponseRule {
    /// Which effects trigger the rule.
    pub on: EffectMatcher,
    /// Events emitted at offsets after the submission.
    pub emit: Vec<(u64, DomainEvent)>,
}

/// Matches submitted effects.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EffectMatcher {
    /// A contract call with this method whose body contains these top-level fields.
    Call {
        /// Method name.
        method: String,
        /// Required body fields and values.
        #[serde(default)]
        fields: BTreeMap<String, JsonValue>,
    },
    /// An unlock (a secret call to `account.unlock`), optionally for one account.
    Unlock {
        /// Required account.
        #[serde(default)]
        account_id: Option<AccountId>,
    },
}

impl EffectMatcher {
    /// Returns whether `effect` matches. `Call` matches plain and secret
    /// calls alike (secrets are never matched). Confirmation requests never
    /// match: they are handled by the app, not the source.
    pub fn matches(&self, effect: &Effect) -> bool {
        let (Some(m), Some(body)) = (effect.method(), effect.body()) else {
            return false;
        };
        match self {
            Self::Call { method, fields } => {
                method == m && fields.iter().all(|(k, v)| body.get(k) == Some(v))
            }
            Self::Unlock { account_id } => {
                m == methods::ACCOUNT_UNLOCK
                    && account_id.as_ref().is_none_or(|want| {
                        body.get("account_id") == Some(&JsonValue::from(want.as_str()))
                    })
            }
        }
    }
}

/// Failure to load a scenario.
#[derive(Debug, Error)]
pub enum ScenarioError {
    /// The file could not be read.
    #[error("cannot read scenario {path}: {source}")]
    Io {
        /// File path.
        path: PathBuf,
        /// Cause.
        #[source]
        source: std::io::Error,
    },
    /// The file is not a valid scenario.
    #[error("invalid scenario: {0}")]
    Parse(#[from] ron::error::SpannedError),
}

impl Scenario {
    /// Parses a scenario from RON text.
    ///
    /// # Errors
    ///
    /// Returns [`ScenarioError::Parse`] on invalid RON or schema.
    pub fn from_ron(text: &str) -> Result<Self, ScenarioError> {
        Ok(ron::from_str(text)?)
    }

    /// Loads a scenario file.
    ///
    /// # Errors
    ///
    /// Returns [`ScenarioError`] when the file is unreadable or invalid.
    pub fn load(path: &Path) -> Result<Self, ScenarioError> {
        let text = std::fs::read_to_string(path)
            .map_err(|source| ScenarioError::Io { path: path.to_owned(), source })?;
        Self::from_ron(&text)
    }
}

/// Directory holding the bundled `*.ron` scenarios.
pub fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/scenarios")
}

/// Names of the renderer conformance fixtures, by level (L0, L1, L2).
pub const CONFORMANCE_NAMES: [&str; 3] = ["l0_basics", "l1_documents", "l2_interactive"];

/// Directory holding the renderer conformance fixtures.
pub fn conformance_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/conformance")
}

/// Loads a renderer conformance fixture by name, e.g. `"l0_basics"`.
///
/// # Errors
///
/// Returns [`ScenarioError`] when the file is missing or invalid.
pub fn load_conformance(name: &str) -> Result<Scenario, ScenarioError> {
    Scenario::load(&conformance_dir().join(format!("{name}.ron")))
}

/// Loads a bundled scenario by name, e.g. `"s03_intervention"`.
///
/// # Errors
///
/// Returns [`ScenarioError`] when the file is missing or invalid.
pub fn load_fixture(name: &str) -> Result<Scenario, ScenarioError> {
    Scenario::load(&fixtures_dir().join(format!("{name}.ron")))
}
