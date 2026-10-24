// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! One scope's task: snapshot, live events, polling, effects, reconnects.
//!
//! The task is the single owner of its translator. Requests run as spawned
//! futures on the scope's multiplexed connection and report back through an
//! internal channel, so several requests of one scope are in flight at once
//! on one connection while translation stays sequential and deterministic (a
//! snapshot is translated in its fixed read order, whatever order the
//! answers arrive in). Every request ends by its deadline, so no effect or
//! read stays pending: a missed deadline is a `Timeout` failure.
//!
//! A listing is a snapshot of the daemon at the time it was read, and
//! answers on the shared connection arrive as they finish: an older listing
//! can arrive after a newer one. Each listing's first page is numbered when
//! it is sent, and an answer older than the listing last applied is dropped,
//! so a listing never undoes a newer one (a conversation just started and
//! listed is not removed by a listing read before the start).
//!
//! Lifecycle: attach, then snapshot. When every read answered the scope is
//! live: an account follows the daemon's event stream from its cursor (see
//! [`super::live`]), and polls only as a fallback; the daemon scope polls
//! the daemon status and account list. A lost connection reports
//! `Disconnected` and re-snapshots after a back-off.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc as std_mpsc;
use std::time::Duration;

use ramflux_desktop_core::{ContextId, DomainEvent, Effect, MomentId, SourceStatus};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio::time::Instant;

use super::live::{SyncState, last_page};
use super::{Delivery, now};
use crate::bus::BusError;
use crate::bus::client::BusClient;
use crate::bus::stream::Streamed;
use crate::bus::subscription::{self, Pushed};
use crate::capabilities;
use crate::config::SdkConfig;
use crate::cursor::CursorStore;
use crate::pins::PinStore;
use crate::plan::fetch::{Fetch, Paged};
use crate::translate::{Scope, Translator};

/// What the bridge forwards to a scope.
pub(crate) enum ScopeCommand {
    /// Perform an effect.
    Effect(Box<Effect>),
    /// The user looks at a context.
    Focus(Option<ContextId>),
    /// A view needs these messages' bodies again.
    Bodies(Vec<MomentId>),
}

/// A read's answer, with the listing number of a first page.
pub(super) type Read = (Fetch, Option<u64>, Result<Value, BusError>);

pub(super) enum Internal {
    Snapshot(Vec<Read>),
    Fetched(Read),
    PollDone,
    EffectDone { method: String, body: Value, result: Result<Value, BusError>, refresh: Vec<Fetch> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Link {
    Attaching,
    Live,
    Lost,
    Locked,
}

pub(super) struct Task {
    pub(super) config: SdkConfig,
    pub(super) scope: Scope,
    pub(super) client: BusClient,
    pub(super) translator: Translator,
    pub(super) pins: PinStore,
    pub(super) outbox: std_mpsc::Sender<Delivery>,
    pub(super) internal: mpsc::UnboundedSender<Internal>,
    pub(super) pushed: mpsc::UnboundedSender<Pushed>,
    pub(super) streamed: mpsc::UnboundedSender<Streamed>,
    pub(super) cursors: CursorStore,
    pub(super) sync: SyncState,
    /// The push subscription (polling fallback only).
    pub(super) push_task: Option<tokio::task::AbortHandle>,
    pub(super) link: Link,
    pub(super) subscribed: bool,
    pub(super) subscribe_at: Instant,
    pub(super) polls_in_flight: usize,
    pub(super) poll_changed: bool,
    pub(super) interval: Duration,
    pub(super) next_poll: Instant,
    pub(super) reconnect_at: Instant,
    pub(super) alive: bool,
    /// Numbers listings' first pages in the order they are sent.
    pub(super) listings_sent: Arc<AtomicU64>,
    /// The number of the listing last applied, per list.
    pub(super) listing_applied: BTreeMap<Paged, u64>,
}

/// Sends one read; a listing's first page gets the next listing number
/// just before it is sent.
async fn read(client: &BusClient, fetch: Fetch, account: Option<&str>, sent: &AtomicU64) -> Read {
    let request = fetch.request(account);
    let number = matches!(fetch, Fetch::Page(_, None)).then(|| sent.fetch_add(1, Ordering::SeqCst));
    let result = client.call(&request).await;
    (fetch, number, result)
}

/// Runs one scope until its command channel closes or its source is gone.
pub(crate) async fn run(
    config: SdkConfig,
    scope: Scope,
    outbox: std_mpsc::Sender<Delivery>,
    mut commands: mpsc::UnboundedReceiver<ScopeCommand>,
) {
    let (internal, mut internal_rx) = mpsc::unbounded_channel();
    let (pushed, mut pushed_rx) = mpsc::unbounded_channel();
    let (streamed, mut streamed_rx) = mpsc::unbounded_channel();
    let socket = config.socket_path.to_string_lossy().into_owned();
    let cursors = CursorStore::new(config.settings_path.clone(), socket.clone());
    let cursor = scope.account().and_then(|account| cursors.load(account));
    let client =
        BusClient::new(config.socket_path.clone(), config.connect_timeout, config.deadlines);
    let start = Instant::now();
    let pins = PinStore::new(config.settings_path.clone());
    let mut translator = Translator::new(scope.clone(), socket);
    if let Some(recorded) = scope.account().and_then(|account| pins.load(account)) {
        translator.set_pins(recorded);
    }
    let mut task = Task {
        interval: config.poll_fast,
        translator,
        pins,
        config,
        scope,
        client,
        outbox,
        internal,
        pushed,
        streamed,
        cursors,
        sync: SyncState::new(cursor),
        push_task: None,
        link: Link::Attaching,
        subscribed: false,
        subscribe_at: start,
        polls_in_flight: 0,
        poll_changed: false,
        next_poll: start,
        reconnect_at: start,
        alive: true,
        listings_sent: Arc::new(AtomicU64::new(0)),
        listing_applied: BTreeMap::new(),
    };
    task.emit(vec![DomainEvent::CapabilitiesUpdated(capabilities::capabilities())]);
    task.show_sync(true);
    task.snapshot();
    while task.alive {
        let wake = task.next_wake();
        tokio::select! {
            command = commands.recv() => match command {
                Some(command) => task.command(command),
                None => break,
            },
            Some(internal) = internal_rx.recv() => task.internal(internal),
            Some(pushed) = pushed_rx.recv() => task.on_pushed(pushed),
            Some(streamed) = streamed_rx.recv() => {
                task.on_streamed(streamed);
                // Apply what else has arrived before the reads it calls for
                // start, so a burst (a replay) is coalesced.
                while task.alive {
                    let Ok(streamed) = streamed_rx.try_recv() else { break };
                    task.on_streamed(streamed);
                }
            }
            () = tokio::time::sleep_until(wake) => task.tick(),
        }
    }
}

impl Drop for Task {
    fn drop(&mut self) {
        self.save_cursor(true);
        self.sync.stop();
        self.stop_push_subscription();
    }
}

impl Task {
    pub(super) fn account(&self) -> Option<String> {
        self.scope.account().map(ToString::to_string)
    }

    /// Delivers events; returns whether there were any.
    pub(super) fn emit(&mut self, events: Vec<DomainEvent>) -> bool {
        let any = !events.is_empty();
        let at = now();
        for event in events {
            if self.outbox.send(Delivery { at, event }).is_err() {
                self.alive = false;
                break;
            }
        }
        any
    }

    fn next_wake(&self) -> Instant {
        let far = Instant::now() + Duration::from_secs(3_600);
        match self.link {
            Link::Lost => self.reconnect_at,
            Link::Live => {
                let polls = self.polls();
                let mut wake =
                    if polls && self.polls_in_flight == 0 { self.next_poll } else { far };
                if polls && self.scope.account().is_some() && !self.subscribed {
                    wake = wake.min(self.subscribe_at);
                }
                if let Some(open_at) = self.sync.open_at {
                    wake = wake.min(open_at);
                }
                wake
            }
            Link::Attaching | Link::Locked => far,
        }
    }

    fn tick(&mut self) {
        let now = Instant::now();
        match self.link {
            Link::Lost if now >= self.reconnect_at => self.snapshot(),
            Link::Live => {
                if self.sync.open_at.is_some_and(|at| now >= at) {
                    self.open_stream();
                }
                let polls = self.polls();
                if polls
                    && self.scope.account().is_some()
                    && !self.subscribed
                    && now >= self.subscribe_at
                {
                    self.subscribe();
                }
                if polls && self.polls_in_flight == 0 && now >= self.next_poll {
                    self.poll();
                }
            }
            _ => {}
        }
    }

    fn reads_for_snapshot(&self) -> Vec<Fetch> {
        match &self.scope {
            Scope::Daemon => Fetch::daemon_snapshot(),
            Scope::Account(id) => Fetch::account_snapshot(id),
        }
    }

    fn snapshot(&mut self) {
        self.link = Link::Attaching;
        let reads = self.reads_for_snapshot();
        let client = self.client.clone();
        let account = self.account();
        let internal = self.internal.clone();
        let sent = self.listings_sent.clone();
        tokio::spawn(async move {
            let mut set = JoinSet::new();
            for (index, fetch) in reads.into_iter().enumerate() {
                let (client, account, sent) = (client.clone(), account.clone(), sent.clone());
                set.spawn(
                    async move { (index, read(&client, fetch, account.as_deref(), &sent).await) },
                );
            }
            let mut results = Vec::new();
            while let Some(Ok(answer)) = set.join_next().await {
                results.push(answer);
            }
            results.sort_by_key(|(index, _)| *index);
            let results = results.into_iter().map(|(_, read)| read).collect();
            let _ = internal.send(Internal::Snapshot(results));
        });
    }

    fn subscribe(&mut self) {
        let Some(account) = self.account() else { return };
        self.subscribed = true;
        let socket = self.config.socket_path.clone();
        let (connect, deadlines) = (self.config.connect_timeout, self.config.deadlines);
        let out = self.pushed.clone();
        let task = tokio::spawn(async move {
            subscription::run(&socket, account, connect, deadlines, out).await;
        });
        self.push_task = Some(task.abort_handle());
    }

    /// Stops the push subscription (the stream took over, or the scope
    /// ends).
    pub(super) fn stop_push_subscription(&mut self) {
        if let Some(task) = self.push_task.take() {
            task.abort();
        }
        self.subscribed = false;
    }

    pub(super) fn lost(&mut self) {
        if self.link != Link::Lost {
            self.emit(vec![DomainEvent::SourceStatusChanged(SourceStatus::Disconnected)]);
        }
        self.link = Link::Lost;
        self.stop_push_subscription();
        self.sync.stop();
        if self.sync.mode == ramflux_desktop_core::SyncMode::Stream {
            self.sync.mode = ramflux_desktop_core::SyncMode::Connecting;
            self.show_sync(true);
        }
        self.reconnect_at = Instant::now() + self.config.reconnect_backoff;
    }

    pub(super) fn locked(&mut self) {
        self.link = Link::Locked;
        self.stop_push_subscription();
        self.sync.stop();
        self.save_cursor(true);
        let events = self.translator.own_account_locked();
        self.emit(events);
    }

    /// Marks the persisted pins on contexts that are known now.
    fn restore_pins(&mut self) {
        let events = self.translator.restore_pins();
        self.persist_pins();
        self.emit(events);
    }

    /// Saves the pins when they changed (a local pin, or one the daemon
    /// reported). Called before the change is emitted: once the store shows
    /// a pin order, the settings file already holds it (a reader of the
    /// file, or a restart right after, never sees an older order).
    pub(super) fn persist_pins(&mut self) {
        if let Some(account) = self.scope.account()
            && self.translator.take_pins_changed()
        {
            self.pins.save(account, self.translator.pins());
        }
    }

    pub(super) fn spawn_fetches(&self, fetches: Vec<Fetch>) {
        for fetch in fetches {
            let (client, account) = (self.client.clone(), self.account());
            let (internal, sent) = (self.internal.clone(), self.listings_sent.clone());
            tokio::spawn(async move {
                let read = read(&client, fetch, account.as_deref(), &sent).await;
                let _ = internal.send(Internal::Fetched(read));
            });
        }
    }

    fn poll(&mut self) {
        let mut sequences: Vec<Vec<Fetch>> = match &self.scope {
            Scope::Daemon => vec![vec![Fetch::DaemonStatus, Fetch::AccountList]],
            // Directories, approvals and contacts (incoming contact requests
            // are recorded by the daemon when it receives them; they are not
            // pushed). Further pages follow on their own.
            Scope::Account(_) => Fetch::account_poll().into_iter().map(|f| vec![f]).collect(),
        };
        sequences.extend(self.translator.poll_reads());
        self.polls_in_flight = sequences.len();
        self.poll_changed = false;
        for sequence in sequences {
            let client = self.client.clone();
            let account = self.account();
            let internal = self.internal.clone();
            let sent = self.listings_sent.clone();
            tokio::spawn(async move {
                for fetch in sequence {
                    let (fetch, number, result) =
                        read(&client, fetch, account.as_deref(), &sent).await;
                    // A failed pull from the node (`message.receive`) never
                    // stops the read of what is already stored: the
                    // conversation keeps showing its messages.
                    let stop = match &result {
                        Err(error) => error.is_connection_lost() || !fetch.is_pull(),
                        Ok(_) => false,
                    };
                    let _ = internal.send(Internal::Fetched((fetch, number, result)));
                    if stop {
                        break;
                    }
                }
                let _ = internal.send(Internal::PollDone);
            });
        }
    }

    fn internal(&mut self, internal: Internal) {
        match internal {
            Internal::Snapshot(results) => self.on_snapshot(results),
            Internal::Fetched((fetch, number, result)) => {
                let finished = last_page(&fetch, &result);
                if !self.outdated(&fetch, number, &result) {
                    if self.on_read(&fetch, result) && self.polls_in_flight > 0 {
                        self.poll_changed = true;
                    }
                    if matches!(fetch, Fetch::Page(Paged::Conversations | Paged::Groups, _)) {
                        self.restore_pins();
                    }
                }
                self.read_ended(&fetch, finished);
            }
            Internal::PollDone => {
                self.polls_in_flight = self.polls_in_flight.saturating_sub(1);
                if self.polls_in_flight == 0 {
                    self.interval = if self.poll_changed {
                        self.config.poll_fast
                    } else {
                        (self.interval * 2).min(self.config.poll_slow)
                    };
                    self.next_poll = Instant::now() + self.interval;
                }
            }
            Internal::EffectDone { method, body, result, refresh } => {
                self.on_effect_done(&method, &body, result, refresh);
            }
        }
    }

    fn on_snapshot(&mut self, results: Vec<Read>) {
        let unanswered = |r: &Result<Value, BusError>| {
            r.as_ref().is_err_and(|e| e.is_connection_lost() || *e == BusError::Timeout)
        };
        // A daemon that accepts the connection but answers none of the reads
        // in time is treated as gone (and retried), not as an empty one.
        if results.iter().any(|(_, _, r)| r.as_ref().is_err_and(BusError::is_connection_lost))
            || results.iter().all(|(_, _, r)| unanswered(r))
        {
            self.lost();
            return;
        }
        self.link = Link::Live;
        for (fetch, number, result) in results {
            if !self.outdated(&fetch, number, &result) {
                self.on_read(&fetch, result);
            }
        }
        // Every list is known now, so pins come back in their saved order.
        self.restore_pins();
        if self.link == Link::Live {
            self.interval = self.config.poll_fast;
            self.next_poll = Instant::now() + self.interval;
            self.subscribe_at = Instant::now();
            self.live_after_snapshot();
        }
    }

    /// Whether this answer is a listing older than the one last applied of
    /// its list (it is dropped); a newer one becomes the last applied.
    /// Failures are never dropped.
    fn outdated(
        &mut self,
        fetch: &Fetch,
        number: Option<u64>,
        result: &Result<Value, BusError>,
    ) -> bool {
        let (Fetch::Page(paged, None), Some(number), Ok(_)) = (fetch, number, result) else {
            return false;
        };
        if self.listing_applied.get(paged).is_some_and(|applied| *applied > number) {
            return true;
        }
        self.listing_applied.insert(*paged, number);
        false
    }

    /// Translates one read; returns whether it produced events.
    fn on_read(&mut self, fetch: &Fetch, result: Result<Value, BusError>) -> bool {
        match result {
            Ok(value) => {
                let translated = self.translator.fetched(fetch, &value, now());
                self.spawn_fetches(translated.follow);
                self.emit(translated.events)
            }
            Err(error) if error.is_connection_lost() => {
                self.lost();
                false
            }
            Err(BusError::Daemon { message, .. })
                if matches!(fetch, Fetch::AccountStatus(_))
                    && self.scope.account().is_some()
                    && message.contains("account not open") =>
            {
                self.locked();
                true
            }
            // A refused read (for example a receive while the node is
            // unreachable) changes nothing; the next poll tries again.
            Err(_) => false,
        }
    }

    fn on_pushed(&mut self, pushed: Pushed) {
        match pushed {
            Pushed::Opened => {}
            Pushed::Event { topic, body } => {
                let events = self.translator.pushed(&topic, &body, now());
                if self.emit(events) {
                    self.interval = self.config.poll_fast;
                }
            }
            Pushed::Lost(_) => {
                self.push_task = None;
                self.subscribed = false;
                self.subscribe_at = Instant::now() + self.config.reconnect_backoff;
            }
        }
    }

    fn command(&mut self, command: ScopeCommand) {
        match command {
            ScopeCommand::Focus(context) => {
                if self.polls() {
                    self.next_poll = Instant::now();
                } else if let Some(context) = &context
                    && self.link == Link::Live
                {
                    let reads = self.translator.context_reads(context);
                    self.request_reads(reads);
                }
                self.translator.set_focus(context);
            }
            ScopeCommand::Effect(effect) => self.effect(*effect),
            ScopeCommand::Bodies(moments) => {
                let reads = self.translator.request_bodies(&moments);
                self.spawn_fetches(reads);
            }
        }
    }
}
