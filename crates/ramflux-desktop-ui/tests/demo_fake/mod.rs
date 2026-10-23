// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Two fake daemons on one fake node, for rehearsing the demo without a
//! node: each daemon is a [`FakeBus`] on its own socket answering from the
//! shared [`world::World`]; the reference CLI's step (receiving an unknown
//! conversation) acts on the same world.
//!
//! - `world`: the node, both daemons' accounts, links and messages.
//! - `rows`: the daemon's JSON rows.

#![allow(dead_code)]

mod rows;
pub mod world;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use ramflux_desktop_test::fake_bus::{FakeBus, socket_path};
use serde_json::{Value, json};

use crate::rehearsal::daemon::DemoDaemon;
use world::World;

/// The shared world.
pub type Shared = Arc<Mutex<World>>;

/// One fake daemon.
pub struct FakeDaemon {
    world: Shared,
    side: usize,
    path: PathBuf,
    bus: Option<FakeBus>,
}

impl FakeDaemon {
    /// Daemon `side` (0 or 1) of `world`, serving at a socket named after `name`.
    pub fn start(world: &Shared, side: usize, name: &str) -> Self {
        let mut daemon = Self { world: world.clone(), side, path: socket_path(name), bus: None };
        daemon.serve();
        daemon
    }

    fn serve(&mut self) {
        let (world, side) = (self.world.clone(), self.side);
        self.bus = Some(FakeBus::start(&self.path, move |method, account, body| {
            world.lock().expect("world").handle(side, method, account, body)
        }));
    }
}

impl Drop for FakeDaemon {
    fn drop(&mut self) {
        if let Some(bus) = self.bus.take() {
            bus.shutdown();
        }
    }
}

impl DemoDaemon for FakeDaemon {
    fn socket(&self) -> PathBuf {
        self.path.clone()
    }

    fn restart(&mut self) -> Result<(), String> {
        if let Some(bus) = self.bus.take() {
            bus.shutdown();
        }
        self.world.lock().expect("world").restart(self.side);
        self.serve();
        Ok(())
    }

    fn account_status(&self, account: &str) -> Result<Value, String> {
        match self.world.lock().expect("world").handle(
            self.side,
            "account.status",
            Some(account),
            &json!({}),
        ) {
            ramflux_desktop_test::fake_bus::Reply::Ok(status) => Ok(status),
            other => Err(format!("account status: {other:?}")),
        }
    }

    fn node_public_key(&self, node: &str) -> Result<Option<String>, String> {
        Ok(self.world.lock().expect("world").node_key(self.side, node))
    }

    fn dm_read(&self, account: &str, conversation: &str) -> Result<(), String> {
        self.world.lock().expect("world").receive(self.side, account, conversation);
        Ok(())
    }
}
