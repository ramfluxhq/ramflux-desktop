// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! How an account scope stays up to date: the daemon's event stream, or
//! polling when the stream is unavailable and the fallback is on.
//!
//! - **Stream.** After the snapshot the scope subscribes from its cursor
//!   (the last entry it applied, kept across restarts). Each entry is
//!   applied once: an entry at or below the cursor is a duplicate and is
//!   dropped, so a replay after a reconnect changes nothing it already
//!   showed. An entry calls for the reads that refresh what it names; reads
//!   of one list are coalesced, and a list that changes while it is being
//!   read is read once more afterwards.
//! - **Gap.** A cursor the daemon no longer covers (`resync_required`), or
//!   no cursor at all: the scope subscribes from the head first, then
//!   reloads everything through the usual reads, so nothing that happens
//!   in between is missed.
//! - **Lag, lost stream.** The scope resubscribes from its cursor; the
//!   daemon replays what it missed.
//! - **Fallback.** A daemon that refuses the stream (an older daemon), or a
//!   stream that keeps failing: with the fallback on, the scope polls and
//!   opens the older push subscription; with it off, it shows that it is
//!   not kept up to date. A failing stream is tried again later; a refused
//!   one only after a reconnect.
//! - **Lock.** The stream stops; a new source resumes from the cursor.
//!
//! What the scope does is reported as `SyncStatusChanged`, together with
//! the daemon's own background sync (`sync.status`).

use std::collections::BTreeSet;
use std::time::Duration;

use ramflux_desktop_core::{DomainEvent, SyncMode, SyncStatus};
use serde_json::Value;
use tokio::time::Instant;

use super::scope::Task;
use crate::bus::BusError;
use crate::bus::stream::{self, StreamEnd, Streamed, Subscription};
use crate::plan::fetch::Fetch;
use crate::translate::stream::{DaemonSync, locks, topics};

/// Failed stream attempts in a row before the scope falls back.
pub(super) const STREAM_ATTEMPTS: u32 = 3;

/// Least time between two writes of the cursor to the settings file.
const CURSOR_SAVE_EVERY: Duration = Duration::from_secs(2);

/// Least time between two reports that differ only in cursor or last run.
const STATUS_EVERY: Duration = Duration::from_secs(5);

/// The stream and polling state of an account scope.
pub(super) struct SyncState {
    pub(super) mode: SyncMode,
    pub(super) reason: Option<String>,
    /// The last stream entry applied.
    pub(super) cursor: Option<u64>,
    /// The cursor last written to the settings file.
    saved: Option<u64>,
    saved_at: Instant,
    /// Subscribe from the head and reload once the stream is open.
    reload_on_open: bool,
    /// Failed attempts since the stream was last open.
    failures: u32,
    /// The running stream connection.
    task: Option<tokio::task::AbortHandle>,
    /// When to open the stream; `None` while it is open or not wanted.
    pub(super) open_at: Option<Instant>,
    /// The cursor when the stream last opened (to tell lag from no progress).
    opened_with: Option<u64>,
    daemon: Option<DaemonSync>,
    shown: Option<SyncStatus>,
    shown_at: Instant,
    /// Lists being read for the stream (first-page key) and those that
    /// changed meanwhile.
    reading: BTreeSet<Fetch>,
    stale: BTreeSet<Fetch>,
    /// Duplicates dropped (diagnostics and tests).
    pub(super) duplicates: u64,
}

impl SyncState {
    pub(super) fn new(cursor: Option<u64>) -> Self {
        let now = Instant::now();
        Self {
            mode: SyncMode::Connecting,
            reason: None,
            cursor,
            saved: cursor,
            saved_at: now,
            reload_on_open: false,
            failures: 0,
            task: None,
            open_at: None,
            opened_with: None,
            daemon: None,
            shown: None,
            shown_at: now,
            reading: BTreeSet::new(),
            stale: BTreeSet::new(),
            duplicates: 0,
        }
    }

    fn status(&self) -> SyncStatus {
        SyncStatus {
            mode: self.mode,
            reason: self.reason.clone(),
            cursor: self.cursor,
            daemon_sync: self.daemon.and_then(|d| d.state),
            last_run: self.daemon.and_then(|d| d.last_run),
            unrouted: self.daemon.and_then(|d| d.unrouted),
        }
    }

    /// Stops the stream connection, if any.
    pub(super) fn stop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
        self.open_at = None;
    }
}

/// The key under which a read is coalesced: a paged list by its first page.
fn read_key(fetch: &Fetch) -> Fetch {
    match fetch {
        Fetch::Page(paged, _) => Fetch::first(*paged),
        Fetch::Surfaces(context, _) => Fetch::Surfaces(context.clone(), None),
        other => other.clone(),
    }
}

impl Task {
    /// Whether this scope polls: the daemon scope always, an account scope
    /// only as the fallback.
    pub(super) fn polls(&self) -> bool {
        self.scope.account().is_none() || self.sync.mode == SyncMode::Polling
    }

    /// The scope is live after a snapshot: follow the stream (or keep
    /// polling) and show the watched contexts' messages.
    pub(super) fn live_after_snapshot(&mut self) {
        if self.scope.account().is_none() {
            return;
        }
        if self.sync.task.is_none() {
            self.sync.open_at = Some(Instant::now());
        }
        if !self.polls() {
            let reads = self.translator.watched_reads();
            self.request_reads(reads);
        }
    }

    /// Opens the stream from the cursor (or from the head before a reload).
    pub(super) fn open_stream(&mut self) {
        let Some(account) = self.account() else { return };
        self.sync.stop();
        let since_seq = if self.sync.reload_on_open { None } else { self.sync.cursor };
        if since_seq.is_none() {
            // Without a cursor the daemon starts at its head: reload once it
            // is open, so what happened before is read.
            self.sync.reload_on_open = true;
        }
        let subscription = Subscription { account, topics: topics(), since_seq };
        let socket = self.config.socket_path.clone();
        let (connect, deadlines) = (self.config.connect_timeout, self.config.deadlines);
        let out = self.streamed.clone();
        let task = tokio::spawn(async move {
            stream::run(&socket, subscription, connect, deadlines, out).await;
        });
        self.sync.task = Some(task.abort_handle());
        self.sync.opened_with = self.sync.cursor;
    }

    pub(super) fn on_streamed(&mut self, streamed: Streamed) {
        match streamed {
            Streamed::Opened { head_seq, .. } => self.stream_opened(head_seq),
            Streamed::Event { seq, topic, body } => self.stream_event(seq, &topic, &body),
            Streamed::Ended(end) => self.stream_ended(end),
        }
    }

    fn stream_opened(&mut self, head_seq: u64) {
        let was_polling = self.sync.mode == SyncMode::Polling;
        self.sync.mode = SyncMode::Stream;
        self.sync.reason = None;
        self.sync.failures = 0;
        if was_polling {
            self.stop_push_subscription();
        }
        if std::mem::take(&mut self.sync.reload_on_open) {
            self.sync.cursor = Some(head_seq);
            self.save_cursor(true);
            self.reload();
        }
        self.show_sync(true);
    }

    /// Reads everything again (after a gap in the stream).
    fn reload(&mut self) {
        let mut reads = match &self.scope {
            crate::translate::Scope::Account(id) => Fetch::account_snapshot(id),
            crate::translate::Scope::Daemon => Vec::new(),
        };
        reads.extend(self.translator.watched_reads());
        self.request_reads(reads);
    }

    fn stream_event(&mut self, seq: Option<u64>, topic: &str, body: &Value) {
        if let Some(seq) = seq {
            if self.sync.cursor.is_some_and(|cursor| seq <= cursor) {
                self.sync.duplicates += 1;
                return;
            }
            self.sync.cursor = Some(seq);
        }
        match topic {
            "sync.status" => {
                self.sync.daemon = Some(DaemonSync::from_body(body));
                self.show_sync(false);
            }
            "account.state" => {
                // A `locked` entry may be replayed after the account was
                // unlocked again (a source resuming from a cursor saved
                // before the lock): the daemon's account status decides; it
                // answers "account not open" while locked, which locks.
                if let Some(account) = self.account().filter(|account| locks(body, account)) {
                    let account = ramflux_desktop_core::AccountId::new(account);
                    self.request_reads(vec![Fetch::AccountStatus(account)]);
                }
            }
            _ => {
                let streamed = self.translator.streamed(topic, body);
                self.emit(streamed.events);
                self.request_reads(streamed.follow);
            }
        }
        self.save_cursor(false);
    }

    fn stream_ended(&mut self, end: StreamEnd) {
        self.sync.task = None;
        if self.link != super::scope::Link::Live {
            return;
        }
        let now = Instant::now();
        match end {
            StreamEnd::ResyncRequired => {
                self.sync.reload_on_open = true;
                self.sync.open_at = Some(now);
            }
            StreamEnd::Lagged { .. } => {
                // Resume at once when the stream moved forward; otherwise
                // back off so a stream that cannot keep up does not spin.
                let moved = self.sync.cursor > self.sync.opened_with;
                self.sync.open_at =
                    Some(if moved { now } else { now + self.config.reconnect_backoff });
            }
            StreamEnd::NotOpen => self.locked(),
            StreamEnd::Unsupported(reason) => self.fall_back(reason, false),
            StreamEnd::Lost(error) => {
                self.sync.failures += 1;
                if self.sync.failures >= STREAM_ATTEMPTS {
                    self.fall_back(lost_reason(&error), true);
                } else {
                    self.sync.mode = SyncMode::Connecting;
                    self.sync.open_at = Some(now + self.config.reconnect_backoff);
                    self.show_sync(true);
                }
            }
        }
    }

    /// The stream is unavailable: poll (when allowed) and say why. A
    /// failing stream is tried again after `stream_retry`.
    fn fall_back(&mut self, reason: String, retry: bool) {
        self.sync.stop();
        self.sync.failures = 0;
        self.sync.reason = Some(reason);
        self.sync.open_at = retry.then(|| Instant::now() + self.config.stream_retry);
        if self.config.poll_fallback {
            if self.sync.mode != SyncMode::Polling {
                self.sync.mode = SyncMode::Polling;
                self.interval = self.config.poll_fast;
                self.next_poll = Instant::now();
                self.subscribe_at = Instant::now();
            }
        } else {
            self.sync.mode = SyncMode::Off;
        }
        self.show_sync(true);
    }

    /// Reports the sync status when it changed (a change of only the
    /// cursor or the last run is reported at most every five seconds).
    pub(super) fn show_sync(&mut self, force: bool) {
        if self.scope.account().is_none() {
            return;
        }
        let status = self.sync.status();
        let shown = self.sync.shown.as_ref();
        if shown == Some(&status) {
            return;
        }
        let quiet = shown.is_some_and(|old| {
            let mut old = old.clone();
            old.cursor = status.cursor;
            old.last_run = status.last_run;
            old == status
        });
        if quiet && !force && self.sync.shown_at.elapsed() < STATUS_EVERY {
            return;
        }
        self.sync.shown = Some(status.clone());
        self.sync.shown_at = Instant::now();
        self.emit(vec![DomainEvent::SyncStatusChanged(status)]);
    }

    /// Writes the cursor (at most every two seconds unless `force`).
    pub(super) fn save_cursor(&mut self, force: bool) {
        let (Some(account), Some(cursor)) = (self.scope.account(), self.sync.cursor) else {
            return;
        };
        if self.sync.saved == Some(cursor)
            || (!force && self.sync.saved_at.elapsed() < CURSOR_SAVE_EVERY)
        {
            return;
        }
        self.cursors.save(account, cursor);
        self.sync.saved = Some(cursor);
        self.sync.saved_at = Instant::now();
    }

    /// Starts the reads, coalescing those already running: a list being
    /// read is read once more after that read ends.
    pub(super) fn request_reads(&mut self, reads: Vec<Fetch>) {
        let mut fresh = Vec::new();
        for fetch in reads {
            let key = read_key(&fetch);
            if self.sync.reading.contains(&key) {
                self.sync.stale.insert(key);
            } else {
                self.sync.reading.insert(key);
                fresh.push(fetch);
            }
        }
        self.spawn_fetches(fresh);
    }

    /// A read ended (`finished`: its last page arrived or it failed).
    pub(super) fn read_ended(&mut self, fetch: &Fetch, finished: bool) {
        let key = read_key(fetch);
        if finished && self.sync.reading.remove(&key) && self.sync.stale.remove(&key) {
            self.request_reads(vec![key]);
        }
    }
}

/// Whether a read's answer was its last page.
pub(super) fn last_page(fetch: &Fetch, answer: &Result<Value, BusError>) -> bool {
    match (fetch, answer) {
        (Fetch::Page(..) | Fetch::Surfaces(..), Ok(value)) => {
            value.get("next_cursor").and_then(Value::as_str).is_none_or(str::is_empty)
        }
        _ => true,
    }
}

fn lost_reason(error: &BusError) -> String {
    match error {
        BusError::Timeout => "the event stream did not answer in time".to_owned(),
        other => other.message(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::plan::fetch::Paged;

    #[test]
    fn reads_are_coalesced_by_list() {
        let next = Fetch::Page(Paged::Conversations, Some("c".to_owned()));
        assert_eq!(read_key(&next), Fetch::first(Paged::Conversations));
        assert!(!last_page(&next, &Ok(json!({ "next_cursor": "d" }))));
        assert!(last_page(&next, &Ok(json!({ "next_cursor": null }))));
        assert!(last_page(&Fetch::Grants, &Ok(json!({}))));
        assert!(last_page(&next, &Err(BusError::Timeout)));
    }
}
