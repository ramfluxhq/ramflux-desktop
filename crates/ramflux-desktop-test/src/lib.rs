// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Scenario-driven test support for Ramflux Desktop.
//!
//! - [`Scenario`]: a RON fixture with a timeline of domain events and
//!   response rules that react to submitted effects.
//! - [`MockSource`]: a [`ramflux_desktop_core::Source`] that plays a scenario.
//! - [`local_echo`]: built-in local-echo and audit responses for every scenario.
//! - [`ScenarioPlayer`]: owns a store and a mock source and offers pause,
//!   step, advance, and seek for tests and the developer panel.
//! - `fake_bus` (feature `fake-bus`): a fake daemon local bus for adapter tests.

#![forbid(unsafe_code)]

#[cfg(feature = "fake-bus")]
pub mod fake_bus;
pub mod local_echo;
pub mod mock_source;
pub mod player;
pub mod scenario;

pub use mock_source::{EffectRecord, MockSource};
pub use player::ScenarioPlayer;
pub use scenario::{
    Companion, DEFAULT_EPOCH, EffectMatcher, ResponseRule, SCENARIO_NAMES, Scenario, ScenarioError,
    fixtures_dir, load_fixture,
};
