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
//! Lifecycle: attach, then snapshot. When every read answered the scope is
//! live: an account subscribes to its pushed topics and polls its watched
//! contexts on an adaptive interval. A lost connection reports
//! `Disconnected` and re-snapshots after a back-off. This polling is the
//! transitional path: when the daemon offers a resumable event stream, the
//! poll reads are replaced by a subscription with a cursor, and only the
//! re-snapshot on `resync_required` remains.

use std::sync::mpsc as std_mpsc;
use std::time::Duration;

use ramflux_desktop_core::{ContextId, DomainEvent, Effect, MomentId, SourceStatus};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio::time::Instant;

use super::{Delivery, now};
use crate::bus::BusError;
use crate::bus::client::BusClient;
use crate::bus::subscription::{self, Pushed};
use crate::capabilities;
use crate::config::SdkConfig;
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

pub(super) enum Internal {
    Snapshot(Vec<(Fetch, Result<Value, BusError>)>),
    Fetched(Fetch, Result<Value, BusError>),
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
    pub(super) link: Link,
    pub(super) subscribed: bool,
    pub(super) subscribe_at: Instant,
    pub(super) polls_in_flight: usize,
    pub(super) poll_changed: bool,
    pub(super) interval: Duration,
    pub(super) next_poll: Instant,
    pub(super) reconnect_at: Instant,
    pub(super) alive: bool,
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
    let socket = config.socket_path.to_string_lossy().into_owned();
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
        link: Link::Attaching,
        subscribed: false,
        subscribe_at: start,
        polls_in_flight: 0,
        poll_changed: false,
        next_poll: start,
        reconnect_at: start,
        alive: true,
    };
    task.emit(vec![DomainEvent::CapabilitiesUpdated(capabilities::capabilities())]);
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
            () = tokio::time::sleep_until(wake) => task.tick(),
        }
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
                let mut wake = if self.polls_in_flight == 0 { self.next_poll } else { far };
                if self.scope.account().is_some() && !self.subscribed {
                    wake = wake.min(self.subscribe_at);
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
                if self.scope.account().is_some() && !self.subscribed && now >= self.subscribe_at {
                    self.subscribe();
                }
                if self.polls_in_flight == 0 && now >= self.next_poll {
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
        tokio::spawn(async move {
            let mut set = JoinSet::new();
            for (index, fetch) in reads.into_iter().enumerate() {
                let client = client.clone();
                let request = fetch.request(account.as_deref());
                set.spawn(async move { (index, fetch, client.call(&request).await) });
            }
            let mut results = Vec::new();
            while let Some(Ok(answer)) = set.join_next().await {
                results.push(answer);
            }
            results.sort_by_key(|(index, _, _)| *index);
            let results = results.into_iter().map(|(_, fetch, result)| (fetch, result)).collect();
            let _ = internal.send(Internal::Snapshot(results));
        });
    }

    fn subscribe(&mut self) {
        let Some(account) = self.account() else { return };
        self.subscribed = true;
        let socket = self.config.socket_path.clone();
        let (connect, deadlines) = (self.config.connect_timeout, self.config.deadlines);
        let out = self.pushed.clone();
        tokio::spawn(
            async move { subscription::run(&socket, account, connect, deadlines, out).await },
        );
    }

    pub(super) fn lost(&mut self) {
        if self.link != Link::Lost {
            self.emit(vec![DomainEvent::SourceStatusChanged(SourceStatus::Disconnected)]);
        }
        self.link = Link::Lost;
        self.subscribed = false;
        self.reconnect_at = Instant::now() + self.config.reconnect_backoff;
    }

    fn locked(&mut self) {
        self.link = Link::Locked;
        let events = self.translator.own_account_locked();
        self.emit(events);
    }

    /// Marks the persisted pins on contexts that are known now.
    fn restore_pins(&mut self) {
        let events = self.translator.restore_pins();
        self.emit(events);
        self.persist_pins();
    }

    /// Saves the pins when they changed (a local pin, or one the daemon
    /// reported).
    pub(super) fn persist_pins(&mut self) {
        if let Some(account) = self.scope.account()
            && self.translator.take_pins_changed()
        {
            self.pins.save(account, self.translator.pins());
        }
    }

    pub(super) fn spawn_fetches(&self, fetches: Vec<Fetch>) {
        for fetch in fetches {
            let client = self.client.clone();
            let request = fetch.request(self.account().as_deref());
            let internal = self.internal.clone();
            tokio::spawn(async move {
                let result = client.call(&request).await;
                let _ = internal.send(Internal::Fetched(fetch, result));
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
            tokio::spawn(async move {
                for fetch in sequence {
                    let result = client.call(&fetch.request(account.as_deref())).await;
                    let failed = result.is_err();
                    let _ = internal.send(Internal::Fetched(fetch, result));
                    if failed {
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
            Internal::Fetched(fetch, result) => {
                if self.on_read(&fetch, result) && self.polls_in_flight > 0 {
                    self.poll_changed = true;
                }
                if matches!(fetch, Fetch::Page(Paged::Conversations | Paged::Groups, _)) {
                    self.restore_pins();
                }
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

    fn on_snapshot(&mut self, results: Vec<(Fetch, Result<Value, BusError>)>) {
        let unanswered = |r: &Result<Value, BusError>| {
            r.as_ref().is_err_and(|e| e.is_connection_lost() || *e == BusError::Timeout)
        };
        // A daemon that accepts the connection but answers none of the reads
        // in time is treated as gone (and retried), not as an empty one.
        if results.iter().any(|(_, r)| r.as_ref().is_err_and(BusError::is_connection_lost))
            || results.iter().all(|(_, r)| unanswered(r))
        {
            self.lost();
            return;
        }
        self.link = Link::Live;
        for (fetch, result) in results {
            self.on_read(&fetch, result);
        }
        // Every list is known now, so pins come back in their saved order.
        self.restore_pins();
        if self.link == Link::Live {
            self.interval = self.config.poll_fast;
            self.next_poll = Instant::now() + self.interval;
            self.subscribe_at = Instant::now();
        }
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
                self.subscribed = false;
                self.subscribe_at = Instant::now() + self.config.reconnect_backoff;
            }
        }
    }

    fn command(&mut self, command: ScopeCommand) {
        match command {
            ScopeCommand::Focus(context) => {
                self.translator.set_focus(context);
                self.next_poll = Instant::now();
            }
            ScopeCommand::Effect(effect) => self.effect(*effect),
            ScopeCommand::Bodies(moments) => {
                let reads = self.translator.request_bodies(&moments);
                self.spawn_fetches(reads);
            }
        }
    }
}
