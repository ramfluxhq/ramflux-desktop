// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Keeps the agents roster current while the live app runs (AGENT-IDENTITY-01).
//!
//! Every [`CREATION_POLL`] the requests still waiting for the phone are read
//! (`agent.operation.get`); the bridge keeps one such read in flight per
//! request, so a tick never stacks a second. Every [`WATCH_INTERVAL`], and
//! again when the daemon connection comes back, the changes since the
//! roster's journal cursor are read (`agent.watch`), so a change made from a
//! phone shows here. The task belongs to the store, not to the agents panel:
//! a creation keeps being followed while the panel is closed.

use std::time::Duration;

use gpui_kit::{Context, Task};
use ramflux_desktop_core::{Intent, IntentKind, SourceStatus};
use ramflux_desktop_sdk::plan::agents::{CREATION_POLL, pending_creation_reads};
use ramflux_desktop_sdk::plan::fetch::Fetch;

use super::StoreHandle;

/// How often the roster's changes are read while the daemon is connected.
pub const WATCH_INTERVAL: Duration = Duration::from_secs(30);

/// The periodic agent reads of a live store.
#[derive(Default)]
pub struct AgentSync {
    task: Option<Task<()>>,
    /// Whether the daemon connection was up at the last tick.
    connected: bool,
}

impl AgentSync {
    /// Starts the ticks (once). Each tick reads the pending creations and,
    /// when due, the roster's changes.
    pub fn start(handle: &mut StoreHandle, cx: &mut Context<StoreHandle>) {
        if handle.agent_sync.task.is_some() {
            return;
        }
        let task = cx.spawn(async move |this, cx| {
            let mut since_watch = Duration::ZERO;
            loop {
                cx.background_executor().timer(CREATION_POLL).await;
                since_watch += CREATION_POLL;
                let watch = since_watch >= WATCH_INTERVAL;
                if watch {
                    since_watch = Duration::ZERO;
                }
                if this.update(cx, |handle, cx| handle.agent_tick(watch, cx)).is_err() {
                    break;
                }
            }
        });
        handle.agent_sync.task = Some(task);
    }
}

impl StoreHandle {
    /// One tick: reads the creations still waiting, and the roster's changes
    /// when `watch` is due or the daemon connection has just come back.
    fn agent_tick(&mut self, watch: bool, cx: &mut Context<Self>) {
        let Some(session) = self.session() else { return };
        let store = session.store();
        let connected = store.source_status() == SourceStatus::Connected;
        let reconnected = connected && !self.agent_sync.connected;
        let cursor_known = store.agent_cursor().is_some();
        let requests: Vec<String> = pending_creation_reads(store)
            .into_iter()
            .filter_map(|read| match read {
                Fetch::AgentOperation(request_id) => Some(request_id),
                _ => None,
            })
            .collect();
        self.agent_sync.connected = connected;
        for request_id in requests {
            self.dispatch(Intent::human(IntentKind::AgentOperationRefresh { request_id }), cx);
        }
        if cursor_known && connected && (watch || reconnected) {
            self.dispatch(Intent::human(IntentKind::AgentWatchRefresh), cx);
        }
    }
}
