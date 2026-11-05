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
use std::sync::atomic::{AtomicBool, Ordering};
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
    restart_window: Arc<AtomicBool>,
    history_read: Arc<AtomicBool>,
}

impl FakeDaemon {
    /// Daemon `side` (0 or 1) of `world`, serving at a socket named after `name`.
    pub fn start(world: &Shared, side: usize, name: &str) -> Self {
        let mut daemon = Self {
            world: world.clone(),
            side,
            path: socket_path(name),
            bus: None,
            restart_window: Arc::default(),
            history_read: Arc::default(),
        };
        daemon.serve();
        daemon
    }

    fn serve(&mut self) {
        let (world, side) = (self.world.clone(), self.side);
        let (window, read) = (self.restart_window.clone(), self.history_read.clone());
        self.bus = Some(FakeBus::start(&self.path, move |method, account, body| {
            if window.load(Ordering::SeqCst) && matches!(method, "message.list" | "group.receive") {
                read.store(true, Ordering::SeqCst);
                return ramflux_desktop_test::fake_bus::Reply::Ok(
                    json!({"messages":[],"next_cursor":null}),
                );
            }
            let mut world = world.lock().expect("world");
            let reply = world.handle(side, method, account, body);
            if matches!(
                method,
                "message.submit"
                    | "message.receipt.delivered"
                    | "group.send"
                    | "group.create"
                    | "group.member.add"
                    | "contact.add"
                    | "contact.request"
                    | "contact.accept"
            ) {
                world.announce_changes();
            }
            reply
        }));
        let bus = self.bus.as_ref().expect("fake bus");
        bus.enable_stream();
        self.world.lock().expect("world").set_publisher(side, bus.publisher());
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
        self.restart_window.store(true, Ordering::SeqCst);
        self.history_read.store(false, Ordering::SeqCst);
        self.serve();
        Ok(())
    }

    fn arm_restart_snapshot(&self) {
        self.bus.as_ref().expect("bus").hold_next_account_subscribe();
    }

    fn restart_snapshot_ready(&self) -> Option<bool> {
        self.restart_window.load(Ordering::SeqCst).then(|| {
            self.history_read.load(Ordering::SeqCst)
                && self.bus.as_ref().is_some_and(FakeBus::account_subscribe_held)
        })
    }

    fn release_restart_snapshot(&self) {
        self.restart_window.store(false, Ordering::SeqCst);
        // The durable history becomes visible and its events are published
        // before the held subscription captures its head.
        self.world.lock().expect("world").announce_changes();
        self.bus.as_ref().expect("bus").release_account_subscribe();
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
        let mut world = self.world.lock().expect("world");
        world.receive(self.side, account, conversation);
        world.announce_changes();
        Ok(())
    }
}
