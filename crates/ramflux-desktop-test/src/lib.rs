// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Scenario-driven test support for Ramflux Desktop.
//!
//! - [`Scenario`]: a RON fixture with a timeline of domain events and
//!   response rules that react to submitted effects.
//! - [`MockSource`]: a [`ramflux_desktop_core::Source`] that plays a scenario.
//! - [`local_echo`]: built-in local-echo and audit responses for every scenario.
//! - [`mock_ceremony`]: the daemon side of the born-v3 creation ceremony
//!   (contract-first), for a mock source that answers its methods.
//! - [`mock_recovery`]: the daemon side of the born-v3 recovery ceremony.
//! - [`ScenarioPlayer`]: owns a store and a mock source and offers pause,
//!   step, advance, and seek for tests and the developer panel.
//! - [`scale`]: generated large scenarios for scale tests.
//! - `fake_bus` (feature `fake-bus`): a fake daemon local bus for adapter tests,
//!   with its event journal in `fake_journal`, its agent surfaces (on the
//!   protocol's state machine) in `fake_surfaces`, conversation tasks
//!   (on the protocol's state machine) in `fake_tasks`, its voice calls
//!   (the CALL-01 bus API) in `fake_calls`, its contact requests
//!   (CONTACT-02, contract-first) in `fake_contact_requests`, and full
//!   backup and restore jobs (the BACKUP-01 job API) in `fake_backup`.
//!   The daemon side of the born-v3 creation and recovery ceremonies, on
//!   the same bus, is `fake_custody` (per-connection ceremonies, accounts
//!   that survive a restart).
//! - `live_daemon` (feature `live-daemon`): a real `rf daemon` from the
//!   pinned product revision for live tests.

#![forbid(unsafe_code)]

#[cfg(feature = "fake-bus")]
pub mod fake_backup;
#[cfg(feature = "fake-bus")]
pub mod fake_bus;
#[cfg(feature = "fake-bus")]
pub mod fake_calls;
#[cfg(feature = "fake-bus")]
pub mod fake_contact_requests;
#[cfg(feature = "fake-bus")]
pub mod fake_custody;
#[cfg(feature = "fake-bus")]
pub mod fake_journal;
#[cfg(feature = "fake-bus")]
pub mod fake_profiles;
#[cfg(feature = "fake-bus")]
pub mod fake_surfaces;
#[cfg(feature = "fake-bus")]
pub mod fake_tasks;
#[cfg(feature = "live-daemon")]
pub mod live_daemon;
pub mod local_echo;
pub mod mock_agents;
pub mod mock_ceremony;
pub mod mock_recovery;
pub mod mock_source;
pub mod player;
pub mod scale;
pub mod scenario;

pub use mock_ceremony::MockCeremony;
pub use mock_recovery::MockRecovery;
pub use mock_source::{EffectRecord, MockSource};
pub use player::ScenarioPlayer;
pub use scenario::{
    CONFORMANCE_NAMES, Companion, DEFAULT_EPOCH, EffectMatcher, ResponseRule, SCENARIO_NAMES,
    Scenario, ScenarioError, conformance_dir, fixtures_dir, load_conformance, load_fixture,
};
