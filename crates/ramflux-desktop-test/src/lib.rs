// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Scenario-driven test support for Ramflux Desktop.
//!
//! - [`Scenario`]: a RON fixture with a timeline of domain events and
//!   response rules that react to submitted effects.
//! - [`MockSource`]: a [`ramflux_desktop_core::Source`] that plays a scenario.
//! - [`ScenarioPlayer`]: owns a store and a mock source and offers pause,
//!   step, advance, and seek for tests and the developer panel.

#![forbid(unsafe_code)]

pub mod mock_source;
pub mod player;
pub mod scenario;

pub use mock_source::{EffectRecord, MockSource};
pub use player::ScenarioPlayer;
pub use scenario::{
    DEFAULT_EPOCH, EffectMatcher, ResponseRule, SCENARIO_NAMES, Scenario, ScenarioError,
    fixtures_dir, load_fixture,
};
