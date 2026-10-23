// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! One scope's task: snapshot, live events, polling, effects, reconnects.
//!
//! The task is the single owner of its translator. Requests run as spawned
//! futures on the scope's lane and report back through an internal channel,
//! so several requests of one scope are in flight at once while translation
//! stays sequential and deterministic (a snapshot is translated in its fixed
//! read order, whatever order the answers arrive in).
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

use ramflux_desktop_core::{
    ContextId, DomainEvent, Effect, EffectFailure, FailureCode, SourceStatus,
};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio::time::Instant;

use super::{Delivery, now};
use crate::bus::BusError;
use crate::bus::lane::Lane;
use crate::bus::subscription::{self, Pushed};
use crate::capabilities;
use crate::config::SdkConfig;
use crate::plan::fetch::Fetch;
use crate::plan::{Plan, plan};
use crate::translate::{Scope, Translator};

/// What the bridge forwards to a scope.
pub(crate) enum ScopeCommand {
    /// Perform an effect.
    Effect(Box<Effect>),
    /// The user looks at a context.
    Focus(Option<ContextId>),
}

enum Internal {
    Snapshot(Vec<(Fetch, Result<Value, BusError>)>),
    Fetched(Fetch, Result<Value, BusError>),
    PollDone,
    EffectDone { method: String, body: Value, result: Result<Value, BusError>, refresh: Vec<Fetch> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Link {
    Attaching,
    Live,
    Lost,
    Locked,
}

struct Task {
    config: SdkConfig,
    scope: Scope,
    lane: Lane,
    translator: Translator,
    outbox: std_mpsc::Sender<Delivery>,
    internal: mpsc::UnboundedSender<Internal>,
    pushed: mpsc::UnboundedSender<Pushed>,
    link: Link,
    subscribed: bool,
    subscribe_at: Instant,
    polls_in_flight: usize,
    poll_changed: bool,
    interval: Duration,
    next_poll: Instant,
    reconnect_at: Instant,
    alive: bool,
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
    let lane =
        Lane::spawn(config.socket_path.clone(), config.request_connections, config.request_timeout);
    let start = Instant::now();
    let mut task = Task {
        interval: config.poll_fast,
        translator: Translator::new(scope.clone(), socket),
        config,
        scope,
        lane,
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
    fn account(&self) -> Option<String> {
        self.scope.account().map(ToString::to_string)
    }

    /// Delivers events; returns whether there were any.
    fn emit(&mut self, events: Vec<DomainEvent>) -> bool {
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
        let lane = self.lane.clone();
        let account = self.account();
        let internal = self.internal.clone();
        tokio::spawn(async move {
            let mut set = JoinSet::new();
            for (index, fetch) in reads.into_iter().enumerate() {
                let lane = lane.clone();
                let request = fetch.request(account.as_deref());
                set.spawn(async move { (index, fetch, lane.call(request).await) });
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
        let timeout = self.config.request_timeout;
        let out = self.pushed.clone();
        tokio::spawn(async move { subscription::run(&socket, account, timeout, out).await });
    }

    fn lost(&mut self) {
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

    fn spawn_fetches(&self, fetches: Vec<Fetch>) {
        for fetch in fetches {
            let lane = self.lane.clone();
            let request = fetch.request(self.account().as_deref());
            let internal = self.internal.clone();
            tokio::spawn(async move {
                let result = lane.call(request).await;
                let _ = internal.send(Internal::Fetched(fetch, result));
            });
        }
    }

    fn poll(&mut self) {
        let mut sequences: Vec<Vec<Fetch>> = match &self.scope {
            Scope::Daemon => vec![vec![Fetch::DaemonStatus, Fetch::AccountList]],
            Scope::Account(_) => {
                vec![vec![Fetch::ConversationList], vec![Fetch::GroupList], vec![Fetch::Approvals]]
            }
        };
        sequences.extend(self.translator.poll_reads());
        self.polls_in_flight = sequences.len();
        self.poll_changed = false;
        for sequence in sequences {
            let lane = self.lane.clone();
            let account = self.account();
            let internal = self.internal.clone();
            tokio::spawn(async move {
                for fetch in sequence {
                    let result = lane.call(fetch.request(account.as_deref())).await;
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
        if results.iter().any(|(_, r)| r.as_ref().is_err_and(BusError::is_connection_lost))
            || results.is_empty()
        {
            self.lost();
            return;
        }
        self.link = Link::Live;
        for (fetch, result) in results {
            self.on_read(&fetch, result);
        }
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
                let events = self.translator.pushed(&topic, &body);
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
        }
    }

    fn effect(&mut self, effect: Effect) {
        let planned = plan(self.scope.account(), effect, &self.translator);
        let (method, body) = (planned.method, planned.body);
        match planned.plan {
            Plan::Call { request, refresh } => {
                let lane = self.lane.clone();
                let internal = self.internal.clone();
                tokio::spawn(async move {
                    let result = lane.call(request).await;
                    let _ = internal.send(Internal::EffectDone { method, body, result, refresh });
                });
            }
            Plan::Local => {
                let events = self.translator.local(&method, &body);
                self.emit(events);
            }
            Plan::Diagnostics { path } => {
                let lane = self.lane.clone();
                let internal = self.internal.clone();
                tokio::spawn(async move {
                    let result = write_diagnostics(&lane, &path).await;
                    let _ = internal.send(Internal::EffectDone {
                        method,
                        body,
                        result,
                        refresh: Vec::new(),
                    });
                });
            }
            Plan::Refuse { code, message } => {
                self.emit(vec![DomainEvent::EffectFailed(EffectFailure { method, code, message })]);
            }
        }
    }

    fn on_effect_done(
        &mut self,
        method: &str,
        body: &Value,
        result: Result<Value, BusError>,
        refresh: Vec<Fetch>,
    ) {
        match result {
            Ok(answer) => {
                let events = self.translator.effect_done(method, body, &answer, now());
                self.emit(events);
                self.spawn_fetches(refresh);
                self.interval = self.config.poll_fast;
                if method == ramflux_desktop_core::methods::ACCOUNT_LOCK
                    && self.scope.account().is_some()
                {
                    self.link = Link::Locked;
                }
            }
            Err(error) => {
                let code = match &error {
                    BusError::Daemon { code, .. } => FailureCode::Daemon(code.clone()),
                    BusError::Transport(_) => FailureCode::Transport,
                    BusError::Timeout => FailureCode::Timeout,
                };
                let failure =
                    EffectFailure { method: method.to_owned(), code, message: error.message() };
                self.emit(vec![DomainEvent::EffectFailed(failure)]);
                if error.is_connection_lost() {
                    self.lost();
                }
            }
        }
    }
}

async fn write_diagnostics(lane: &Lane, path: &str) -> Result<Value, BusError> {
    let status = lane.call(Fetch::DaemonStatus.request(None)).await?;
    let nodes = lane.call(Fetch::NodeList.request(None)).await?;
    let text = crate::diagnostics::bundle(&status, &nodes);
    std::fs::write(path, text)
        .map_err(|error| BusError::Daemon { code: "io".to_owned(), message: error.to_string() })?;
    Ok(json!({ "path": path }))
}
