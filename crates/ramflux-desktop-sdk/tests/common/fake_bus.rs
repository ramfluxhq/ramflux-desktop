// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Recorded and SDK-shaped daemon answers for the fake bus.

use std::collections::BTreeMap;

use serde_json::Value;

pub use ramflux_desktop_test::fake_bus::{FakeBus, Reply, socket_path};

/// Recorded and SDK-shaped daemon answers, by method.
pub fn frames() -> BTreeMap<String, Value> {
    let text = include_str!("../fixtures/rfd_frames.json");
    let all: BTreeMap<String, Value> = serde_json::from_str(text).expect("fixture json");
    all.into_iter().filter(|(k, _)| !k.starts_with('_')).collect()
}
