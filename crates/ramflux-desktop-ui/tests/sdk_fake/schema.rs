// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The schema generation the fake daemon reports in `daemon.status`, and
//! whether its accounts are from an older development build (S10c c10).

use std::sync::Mutex;

use ramflux_desktop_test::fake_bus::Reply;
use serde_json::Value;

use super::Latch;

/// What `daemon.status` reports.
#[derive(Clone, Default)]
pub enum Generation {
    /// No `schema_generation` field (a daemon older than p00).
    #[default]
    Absent,
    /// This JSON value.
    Is(Value),
    /// The read fails.
    Fails,
}

/// The fake daemon's schema behaviour.
#[derive(Default)]
pub struct Schema {
    /// Holds `daemon.status` answers while closed.
    pub hold: Latch,
    generation: Mutex<Generation>,
    reset: Mutex<bool>,
}

impl Schema {
    /// Reports `generation` from now on.
    pub fn report(&self, generation: Generation) {
        *self.generation.lock().expect("generation") = generation;
    }

    /// Makes the accounts refuse every read with `SchemaResetRequired`.
    pub fn old_generation(&self, on: bool) {
        *self.reset.lock().expect("reset") = on;
    }

    /// Holds or fails the status read, or refuses an account read.
    pub(crate) fn gate(&self, method: &str, account: Option<&str>) -> Option<Reply> {
        if method == "daemon.status" {
            self.hold.pass();
            if matches!(*self.generation.lock().expect("generation"), Generation::Fails) {
                return Some(Reply::Error("Unavailable", "status unavailable".to_owned()));
            }
        }
        (account.is_some()
            && *self.reset.lock().expect("reset")
            && matches!(method, "conversation.directory" | "group.list" | "object.directory"))
        .then(|| Reply::Error("SchemaResetRequired", "older generation".to_owned()))
    }

    /// Adds the reported generation to a `daemon.status` answer.
    pub(crate) fn patch(&self, frame: &mut Value) {
        if let Generation::Is(value) = &*self.generation.lock().expect("generation") {
            frame["schema_generation"] = value.clone();
        }
    }
}
