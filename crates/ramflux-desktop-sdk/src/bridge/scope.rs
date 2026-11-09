// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! One scope's task: snapshot, live events, effects and reconnects.
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
//! Lifecycle: attach, then snapshot. Local account status establishes
//! readiness before unrelated snapshot reads finish. After the snapshot,
//! an account follows the daemon's event stream from its cursor (see
//! [`super::live`]). Actions, account state events and stream opens refresh
//! the daemon status and account list. A lost connection reports
//! `Disconnected`; ordinary EOF on the daemon scope enters lost-state backoff
//! and independently re-snapshots, including nodes, after reconnect.

#[cfg(test)]
use std::sync::mpsc as std_mpsc;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use ramflux_desktop_core::{ContextId, DomainEvent, Effect, MomentId, SchemaGate, SourceStatus};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio::time::Instant;

use super::gate::{self, Admit, Gate};
use super::live::{SyncState, last_page};
use super::read_recovery::MAX_READS;
use super::{Delivery, now};
use crate::bus::BusError;
use crate::bus::client::BusClient;
use crate::bus::jobs::{JobLink, JobStreamed};
use crate::bus::stream::Streamed;
use crate::bus::subscription::Pushed;
use crate::capabilities;
use crate::config::SdkConfig;
use crate::cursor::CursorStore;
use crate::pins::PinStore;
use crate::plan::fetch::{Fetch, Paged};
use crate::receipts::Receipts;
use crate::translate::{Scope, Translator};

/// What the bridge forwards to a scope.
pub(crate) enum ScopeCommand {
    /// Perform an effect.
    Effect(Box<Effect>),
    /// The user looks at a context.
    Focus(Option<ContextId>),
    /// A view needs these messages' bodies again.
    Bodies(Vec<MomentId>),
    /// An object fetch failed, through the media test seam.
    #[cfg(feature = "test-support")]
    MediaFailed(String, crate::bus::BusError),
    /// Typed v1 updates, through the projection test seam.
    #[cfg(feature = "test-support")]
    Generated(Vec<ramflux_protocol::message::MessageUpdate>),
    /// Reads the daemon status again for the schema gate (tests).
    #[cfg(feature = "test-support")]
    RecheckSchema,
    /// Reports what the gate holds (tests).
    #[cfg(feature = "test-support")]
    GateProbe(std::sync::mpsc::Sender<GateProbe>),
    /// The media engine needs a call's media session.
    CallMedia(
        ramflux_desktop_core::CallId,
        crate::completion::CompletionSender<super::MediaAnswer>,
    ),
    /// Daemon scope only: whether an account scope is attached (its own
    /// link follows the backup jobs then, so the daemon scope's closes).
    AccountsLive(bool),
    RefreshDaemon,
    ConnectionStatus(SourceStatus),
}

/// What the schema gate holds, for tests.
#[cfg(feature = "test-support")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateProbe {
    /// The connection epoch.
    pub epoch: u64,
    /// The verdict.
    pub gate: SchemaGate,
    /// Message-class reads waiting.
    pub queued: usize,
    /// Cached message plaintexts in the translator.
    pub plaintext: usize,
    /// Delivery receipts waiting to be sent.
    pub receipts: usize,
}

/// Where a scope's ceremonies hand the recovery phrase and count it.
pub(crate) struct Ceremonies {
    pub(crate) phrases: crate::completion::CompletionSender<super::Handoff>,
    pub(crate) audit: Arc<super::PhraseAudit>,
}

/// A read's answer, with the listing number of a first page.
pub(super) type Read = (Fetch, Option<u64>, Result<Value, BusError>);

/// Which read attempt, in which gate epoch, produced a completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Stamp {
    pub(super) epoch: u64,
    pub(super) attempt: u64,
}

/// Everything that reaches the scope's inbox. A completion of a read carries
/// the [`Stamp`] it was dispatched under; [`Internal::stamp`] is exhaustive,
/// so a new variant must say whether it is a read, and the one check in
/// `Task::internal` drops a superseded read before any bookkeeping,
/// translation or lock.
pub(super) enum Internal {
    /// A batch of snapshot reads.
    Snapshot(Stamp, Vec<Read>),
    /// The daemon's schema status for the gate, with the generation of the
    /// connection it travelled on.
    SchemaAnswer {
        stamp: Stamp,
        generation: u64,
        result: Result<Value, BusError>,
    },
    /// A single read's completion.
    FetchedAt {
        stamp: Stamp,
        read: Read,
    },
    /// Marks where a new gate epoch began in this channel: whatever was
    /// queued before it belongs to an older epoch.
    EpochStart,
    /// The request connection was opened (or found open) for an attach or a
    /// wake; it decides nothing by itself.
    Attached {
        snapshot: bool,
        result: Result<u64, BusError>,
    },
    SendDone {
        id: String,
        result: Result<Value, BusError>,
    },
    EffectDone {
        method: String,
        body: Value,
        result: Result<Value, BusError>,
        refresh: Vec<Fetch>,
    },
    /// A delivery receipt's answer: its key and whether it was sent.
    ReceiptDone {
        key: String,
        sent: bool,
    },
    /// What a creation or recovery step came to.
    Custody(super::custody::Outcome),
}

impl Internal {
    /// The stamp of a read's completion; `None` for what is not a read.
    fn stamp(&self) -> Option<Stamp> {
        match self {
            Self::Snapshot(stamp, _)
            | Self::SchemaAnswer { stamp, .. }
            | Self::FetchedAt { stamp, .. } => Some(*stamp),
            // Not reads: an attach wake (guarded by `attach_pending`), the
            // epoch marker, and answers to sends, effects, receipts and
            // custody ceremonies, each keyed by its own record. Reads that
            // an effect asks for are spawned when it is handled, under the
            // stamp then current.
            Self::Attached { .. }
            | Self::EpochStart
            | Self::SendDone { .. }
            | Self::EffectDone { .. }
            | Self::ReceiptDone { .. }
            | Self::Custody(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Link {
    Attaching,
    Live,
    Lost,
    Locked,
}

pub(super) struct Task {
    pub(super) sends: BTreeMap<String, super::sends::PendingSend>,
    pub(super) config: SdkConfig,
    pub(super) scope: Scope,
    pub(super) client: BusClient,
    pub(super) translator: Translator,
    pub(super) pins: PinStore,
    pub(super) outbox: crate::completion::CompletionSender<Delivery>,
    pub(super) internal: mpsc::UnboundedSender<Internal>,
    pub(super) streamed: mpsc::UnboundedSender<Streamed>,
    pub(super) cursors: CursorStore,
    pub(super) sync: SyncState,
    /// Legacy push translation retained for compatibility tests; no production
    /// sender starts it. Remove with the legacy subscription API in a follow-up.
    pub(super) push_task: Option<tokio::task::AbortHandle>,
    pub(super) link: Link,
    pub(super) daemon_refresh: Option<mpsc::UnboundedSender<super::DaemonNotice>>,
    /// Per-fetch bounded recovery and cancellable reads.
    pub(super) read_recovery: super::read_recovery::ReadRecovery,
    snapshot_task: Option<tokio::task::AbortHandle>,
    /// The schema gate of this connection (account scopes).
    pub(super) gate: Gate,
    /// What the store was last told of the gate.
    gate_told: (SchemaGate, Option<i64>),
    /// When a failed status read is repeated.
    gate_retry_at: Option<Instant>,
    /// The request-connection generation the current epoch's status was
    /// read on; a newer generation starts a new epoch.
    gate_connection: Option<u64>,
    /// Epoch markers not yet seen in the internal channel: while any is
    /// pending, a completion is an older epoch's.
    pending_markers: u32,
    /// Snapshot batches of this epoch still in flight.
    batch_pending: u32,
    /// Numbers the current read attempt: every cancellation (a new epoch, a
    /// retry in the same epoch, a lost link) starts a new one, and a batch
    /// completion of an earlier attempt is dropped before any bookkeeping,
    /// whatever its epoch.
    read_attempt: u64,
    /// The newest request-connection generation a recovery has been run for.
    /// Recovery is a pure function of the client's monotonic generation
    /// counter: whenever the counter is ahead of this, `recover_connection`
    /// runs (once) and takes the generation it observed. Notifications only
    /// wake that check; they never decide.
    recovered_generation: u64,
    /// A snapshot has asked for a connection and no recovery has run since
    /// (cleared by every recovery, whichever handler ran it).
    attach_pending: bool,
    /// The generation of every recovery run, in order (tests).
    #[cfg(any(test, feature = "test-support"))]
    pub(super) recoveries: Vec<u64>,
    /// Message-class reads sent while the pipeline was not clear (tests).
    #[cfg(any(test, feature = "test-support"))]
    pub(super) guard_violations: u32,
    pub(super) reconnect_at: Instant,
    pub(super) alive: bool,
    /// Numbers listings' first pages in the order they are sent.
    pub(super) listings_sent: Arc<AtomicU64>,
    /// The number of the listing last applied, per list.
    pub(super) listing_applied: BTreeMap<Paged, u64>,
    /// Delivery receipts of received direct messages (account scope).
    pub(super) receipts: Option<Receipts>,
    /// Authenticated renames shown in conversations (account scope).
    pub(super) renames: Option<crate::renames::RenameStore>,
    /// Which person's account custodies each AI participant this desktop
    /// made (remembered across restarts).
    pub(super) custodians: crate::custodians::CustodianStore,
    /// The daemon answered `call.list` (it has the call API, and calls are
    /// not turned off in the settings; until then the call methods are
    /// declared unsupported), whether it refused `contact.requests.list`
    /// as an unknown method (then the request methods are), and whether it
    /// answered `account.backup.jobs` (the full backup job API; until then
    /// the backup methods are declared unsupported). Only an unknown method
    /// says an API is missing; other refusals are read failures.
    pub(super) found: capabilities::Found,
    /// The attended connection with the daemon-level job stream; the job
    /// requests are sent on it.
    pub(super) job_link: JobLink,
    /// Where the job link is, and when to open it.
    pub(super) jobs: super::jobs::Jobs,
    pub(super) jobs_open_at: Option<Instant>,
    /// Daemon scope: an account scope is attached, which follows the jobs
    /// itself; this scope's link stays closed meanwhile.
    pub(super) accounts_live: bool,
    /// The creation and recovery ceremonies of this scope, each on a
    /// connection of its own.
    pub(super) custody: super::custody::Custody,
    /// Revision of the newest applied networking event.
    pub(super) networking_revision: u64,
    /// The request connection reported down and has not reported live since.
    pub(super) transport_disconnected: bool,
    pub(super) ceremonies: Ceremonies,
}

/// Sends one read; a listing's first page gets the next listing number
/// just before it is sent.
async fn read(
    client: &BusClient,
    fetch: Fetch,
    account: Option<&str>,
    sent: &AtomicU64,
    revision: u64,
) -> Read {
    let request = fetch.request(account);
    let number = if matches!(fetch, Fetch::AccountStatus(_)) {
        Some(revision)
    } else {
        matches!(fetch, Fetch::Page(_, None)).then(|| sent.fetch_add(1, Ordering::SeqCst))
    };
    let result = client.call(&request).await;
    (fetch, number, result)
}

/// What a scope's task listens to besides its commands.
pub(super) struct Inbox {
    pub(super) internal: mpsc::UnboundedReceiver<Internal>,
    pub(super) connection_states: mpsc::UnboundedReceiver<crate::bus::client::ConnectionState>,
    pub(super) pushed: mpsc::UnboundedReceiver<Pushed>,
    pub(super) streamed: mpsc::UnboundedReceiver<Streamed>,
    pub(super) jobs: mpsc::UnboundedReceiver<JobStreamed>,
}

/// A scope's task, not started, with what it listens to.
pub(super) fn build(
    config: SdkConfig,
    scope: Scope,
    outbox: impl Into<crate::completion::CompletionSender<Delivery>>,
    ceremonies: Ceremonies,
) -> (Task, Inbox) {
    let outbox = outbox.into();
    let (internal, internal_rx) = mpsc::unbounded_channel();
    let (connection_states, connection_rx) = mpsc::unbounded_channel();
    // Legacy push is intentionally dormant: no sender/task is retained.
    // Account event streams own delivery; remove this compatibility branch in a follow-up.
    let (_pushed, pushed_rx) = mpsc::unbounded_channel();
    let (streamed, streamed_rx) = mpsc::unbounded_channel();
    let (jobs_out, jobs_rx) = mpsc::unbounded_channel::<JobStreamed>();
    let socket = config.socket_path.to_string_lossy().into_owned();
    let cursors = CursorStore::new(config.settings_path.clone(), socket.clone());
    let cursor = scope.account().and_then(|account| cursors.load(account));
    let client = BusClient::with_status(
        config.socket_path.clone(),
        config.connect_timeout,
        config.deadlines,
        Some(connection_states),
    );
    let job_link = JobLink::new(
        config.socket_path.clone(),
        config.connect_timeout,
        config.deadlines,
        jobs_out,
    );
    let start = Instant::now();
    let pins = PinStore::new(config.settings_path.clone());
    let mut translator = Translator::new(scope.clone(), socket.clone());
    translator.set_inline_thumbnails(config.inline_thumbnails);
    if let Some(recorded) = scope.account().and_then(|account| pins.load(account)) {
        translator.set_pins(recorded);
    }
    let receipts = scope.account().map(|account| {
        Receipts::new(config.settings_path.clone(), socket.clone(), account.clone())
    });
    let renames = scope.account().map(|account| {
        crate::renames::RenameStore::new(
            config.settings_path.clone(),
            socket.clone(),
            account.clone(),
        )
    });
    if let Some(renames) = &renames {
        translator.restore_renames(renames.records());
    }
    let custodians = crate::custodians::CustodianStore::new(config.settings_path.clone(), socket);
    translator.restore_custodians(custodians.links());
    let gate = Gate::new(scope.account().is_some(), config.schema_support);
    if gate.enabled() {
        // Fail closed from the start: before any connection has been checked
        // the gate is unresolved and the wire refuses message-class traffic.
        client.approval().enforce();
    }
    let task = Task {
        gate,
        gate_told: (SchemaGate::Open, None),
        gate_retry_at: None,
        gate_connection: None,
        pending_markers: 0,
        batch_pending: 0,
        read_attempt: 0,
        recovered_generation: 0,
        #[cfg(any(test, feature = "test-support"))]
        recoveries: Vec::new(),
        attach_pending: false,
        #[cfg(any(test, feature = "test-support"))]
        guard_violations: 0,
        translator,
        pins,
        config,
        scope,
        client,
        outbox,
        internal,
        streamed,
        cursors,
        sync: SyncState::new(cursor),
        push_task: None,
        link: Link::Attaching,
        daemon_refresh: None,
        read_recovery: super::read_recovery::ReadRecovery::default(),
        snapshot_task: None,
        reconnect_at: start,
        alive: true,
        listings_sent: Arc::new(AtomicU64::new(0)),
        listing_applied: BTreeMap::new(),
        receipts,
        renames,
        custodians,
        found: capabilities::Found::default(),
        job_link,
        jobs: super::jobs::Jobs::Closed,
        jobs_open_at: None,
        accounts_live: false,
        custody: super::custody::Custody::new(),
        sends: BTreeMap::new(),
        networking_revision: 0,
        transport_disconnected: false,
        ceremonies,
    };
    (
        task,
        Inbox {
            internal: internal_rx,
            connection_states: connection_rx,
            pushed: pushed_rx,
            streamed: streamed_rx,
            jobs: jobs_rx,
        },
    )
}

/// Runs one scope until its command channel closes or its source is gone.
pub(crate) async fn run(
    config: SdkConfig,
    scope: Scope,
    outbox: crate::completion::CompletionSender<Delivery>,
    ceremonies: Ceremonies,
    mut commands: mpsc::UnboundedReceiver<ScopeCommand>,
    daemon_refresh: mpsc::UnboundedSender<super::DaemonNotice>,
) {
    let (mut task, inbox) = build(config, scope, outbox, ceremonies);
    task.daemon_refresh = Some(daemon_refresh);
    let Inbox {
        internal: mut internal_rx,
        connection_states: mut connection_rx,
        pushed: mut pushed_rx,
        streamed: mut streamed_rx,
        jobs: mut jobs_rx,
    } = inbox;
    let declared = capabilities::capabilities(capabilities::Found::default());
    task.emit(vec![DomainEvent::CapabilitiesUpdated(declared)]);
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
            Some(status) = connection_rx.recv() => task.connection_changed(status),
            Some(pushed) = pushed_rx.recv() => task.on_pushed(pushed),
            Some(jobs) = jobs_rx.recv() => task.on_jobs(jobs),
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
        self.cancel_reads();
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
    pub(super) fn emit(&mut self, mut events: Vec<DomainEvent>) -> bool {
        // A reply dispatched before EOF cannot make the closed connection live.
        if self.transport_disconnected {
            events.retain(|event| {
                !matches!(event, DomainEvent::SourceStatusChanged(SourceStatus::Connected))
            });
        }
        if let Some(id) = self.scope.account() {
            events.extend(Self::networking_failures(&events, id));
        }
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

    pub(super) fn networking_failures(
        events: &[DomainEvent],
        id: &ramflux_desktop_core::AccountId,
    ) -> Vec<DomainEvent> {
        events
            .iter()
            .filter_map(|event| {
                let DomainEvent::EffectFailed(failure) = event else {
                    return None;
                };
                let ramflux_desktop_core::FailureCode::Daemon(code) = &failure.code else {
                    return None;
                };
                matches!(
                    code.as_str(),
                    "CustodyForkFenced"
                        | "AuthorityUnavailable"
                        | "OperationReconciliationRequired"
                )
                .then(|| DomainEvent::AccountNetworkingFailed {
                    account_id: id.clone(),
                    code: failure.code.clone(),
                })
            })
            .collect()
    }

    pub(super) fn refresh_daemon(&mut self) {
        if self.scope == Scope::Daemon {
            self.spawn_fetches(vec![Fetch::DaemonStatus, Fetch::AccountList]);
        } else if let Some(tx) = &self.daemon_refresh {
            let _ = tx.send(super::DaemonNotice::Refresh);
        }
    }

    /// A fixture that never attaches: an open gate whose check is inert.
    #[cfg(test)]
    pub(super) fn open_gate_for_tests(&mut self) {
        self.gate.begin_epoch();
        let epoch = self.gate.epoch();
        let _ = self.gate.resolve(epoch, Ok(&serde_json::json!({ "schema_generation": 0 })));
        self.client.approval().approve_any();
        self.recovered_generation = u64::MAX;
    }

    /// Begins a connection epoch: the gate's approval is revoked, what it
    /// held is dropped, and the daemon is asked for its schema generation.
    pub(super) fn begin_gate_epoch(&mut self) {
        if !self.gate.enabled() {
            return;
        }
        self.client.approval().enforce();
        self.client.approval().revoke();
        let epoch = self.gate.begin_epoch();
        self.gate_retry_at = None;
        self.gate_connection = None;
        self.batch_pending = 0;
        // Everything queued in the channel before this marker is an older
        // epoch's: its completions are dropped untranslated.
        self.pending_markers += 1;
        let _ = self.internal.send(Internal::EpochStart);
        if let Some(receipts) = &mut self.receipts {
            receipts.drop_pending();
        }
        self.gate_changed();
        self.spawn_schema_status(epoch);
    }

    /// Reads the daemon status for the gate of `epoch`, remembering which
    /// connection carried it.
    fn spawn_schema_status(&self, epoch: u64) {
        let (client, internal) = (self.client.clone(), self.internal.clone());
        let request = Fetch::SchemaStatus(epoch).request(None);
        let stamp = Stamp { epoch, attempt: self.read_attempt };
        tokio::spawn(async move {
            let (generation, result) = client.call_on_connection(&request).await;
            let _ = internal.send(Internal::SchemaAnswer { stamp, generation, result });
        });
    }

    /// Reads the daemon status again in this epoch; a different generation
    /// re-evaluates the gate.
    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn recheck_schema(&mut self) {
        if self.gate.enabled() && !matches!(self.link, Link::Locked | Link::Lost) {
            self.spawn_schema_status(self.gate.epoch());
        }
    }

    /// A status read for the gate came back.
    fn on_schema_status(&mut self, epoch: u64, generation: u64, result: &Result<Value, BusError>) {
        // An answer of an older epoch, or one that travelled on a connection
        // that is no longer the current one, decides nothing.
        if self.pending_markers > 0 || epoch != self.gate.epoch() {
            return;
        }
        if generation > 0 && generation != self.client.connections_opened() {
            // The connection changed under the status read (its answer or its
            // failure is the old connection's). It decides nothing: the
            // generation check recovers the newer connection on its own.
            return;
        }
        if generation > 0 {
            self.gate_connection = Some(generation);
        }
        let answer = result.as_ref().map_err(|_| ());
        if !self.gate.resolve(epoch, answer) {
            return;
        }
        if self.gate.verdict() == SchemaGate::Failed {
            self.gate_retry_at = Some(Instant::now() + self.config.reconnect_backoff);
        } else {
            self.gate_retry_at = None;
        }
        if self.gate.is_open() {
            // Messages may travel on this connection, and only on it.
            self.client.approval().approve(generation);
        } else {
            self.client.approval().revoke();
        }
        self.gate_changed();
    }

    /// Tells the store the gate's verdict when it changed; closing drops what
    /// was remembered of message content, opening releases what was held.
    fn gate_changed(&mut self) {
        if !self.gate.is_open() {
            self.client.approval().revoke();
        }
        let told = (self.gate.verdict(), self.gate.generation());
        if told == self.gate_told {
            return;
        }
        let was_open = self.gate_told.0.is_open();
        self.gate_told = told;
        if let Some(account) = self.scope.account().cloned() {
            self.emit(vec![DomainEvent::SchemaGateChanged {
                account_id: account,
                gate: told.0,
                generation: told.1,
            }]);
        }
        if was_open && !told.0.is_open() {
            self.translator.drop_message_state();
            if let Some(receipts) = &mut self.receipts {
                receipts.drop_pending();
            }
        } else if !was_open && told.0.is_open() {
            self.flush_gate();
        }
    }

    /// Sends what the gate held, in order, once it is open and the
    /// prerequisite reads (identities, devices, ...) that messages depend on
    /// are in. Called wherever either of those can become true.
    fn flush_gate(&mut self) {
        if !self.gate.pipeline_clear() {
            return;
        }
        let (queue, dropped) = self.gate.release();
        if !queue.is_empty() || dropped {
            self.run_release(queue, dropped);
        }
    }

    fn run_release(&mut self, queue: Vec<(Fetch, bool)>, dropped: bool) {
        let (batch, single): (Vec<_>, Vec<_>) = queue.into_iter().partition(|(_, batch)| *batch);
        self.run_batch(batch.into_iter().map(|(fetch, _)| fetch).collect());
        self.spawn_fetches(single.into_iter().map(|(fetch, _)| fetch).collect());
        if dropped {
            // Reads were dropped while closed (or their results rejected):
            // the messages are read again.
            let mut again: Vec<Fetch> =
                self.reads_for_snapshot().into_iter().filter(gate::is_message_read).collect();
            again.extend(self.translator.watched_reads());
            self.request_reads(again);
        }
    }

    /// Repeats a failed status read when its backoff is over.
    fn retry_gate(&mut self, now: Instant) {
        if self.gate_retry_at.is_some_and(|at| now >= at) {
            self.gate_retry_at = None;
            self.gate.retry();
            self.gate_changed();
            self.spawn_schema_status(self.gate.epoch());
        }
    }

    pub(super) fn connection_status(&mut self, status: SourceStatus) {
        // Initial readiness still comes from account.status, not a subscription.
        if status == SourceStatus::Connected && !self.transport_disconnected {
            return;
        }
        self.transport_disconnected = status == SourceStatus::Disconnected;
        if status == SourceStatus::Connected
            && self.scope.account().is_some()
            && self.link == Link::Live
        {
            // A recovered connection may be a new daemon with changed local
            // state even when its cursor is still valid. This only wakes the
            // generation check: the request connection is opened if it is
            // dead, and the counter, not this notification, decides.
            self.observe_connection();
            self.spawn_attach(false);
        }
        self.emit(vec![DomainEvent::SourceStatusChanged(status)]);
        if self.scope.account().is_some()
            && let Some(tx) = &self.daemon_refresh
        {
            let _ = tx.send(super::DaemonNotice::Status(status));
        }
    }

    pub(super) fn connection_changed(&mut self, status: crate::bus::client::ConnectionState) {
        self.observe_connection();
        if status.generation != self.client.connections_opened() {
            return;
        }
        if status.connected {
            if self.transport_disconnected {
                self.connection_status(SourceStatus::Connected);
            }
        } else {
            self.transport_disconnected = true;
            if self.scope == Scope::Daemon {
                // No account/job stream is required to recover the daemon.
                self.lost();
            } else {
                self.connection_status(SourceStatus::Disconnected);
            }
        }
    }

    fn next_wake(&self) -> Instant {
        let far =
            self.next_send_due().unwrap_or_else(|| Instant::now() + Duration::from_secs(3_600));
        let far = self.gate_retry_at.map_or(far, |at| far.min(at));
        match self.link {
            Link::Lost => self.reconnect_at.min(far),
            Link::Live => {
                let mut wake = far;
                if let Some(open_at) = self.sync.open_at {
                    wake = wake.min(open_at);
                }
                if let Some(open_at) = self.jobs_open_at {
                    wake = wake.min(open_at);
                }
                if let Some(due) = self.read_recovery.next_due() {
                    wake = wake.min(due);
                }
                if let Some(due) = self.receipts.as_ref().and_then(Receipts::next_due) {
                    wake = wake.min(due);
                }
                wake
            }
            Link::Attaching | Link::Locked => far,
        }
    }

    fn tick(&mut self) {
        self.observe_connection();
        self.retry_sends(false);
        let now = Instant::now();
        if !matches!(self.link, Link::Lost | Link::Locked) {
            self.retry_gate(now);
        }
        match self.link {
            Link::Lost if now >= self.reconnect_at => self.snapshot(),
            Link::Live => {
                if self.sync.open_at.is_some_and(|at| now >= at) {
                    self.open_stream();
                }
                if self.jobs_open_at.is_some_and(|at| now >= at) {
                    self.open_jobs();
                }
                let retries = self.read_recovery.due(now);
                self.spawn_reads(retries, false);
                self.send_receipts();
            }
            _ => {}
        }
    }

    pub(super) fn reads_for_snapshot(&self) -> Vec<Fetch> {
        match &self.scope {
            Scope::Daemon => {
                let mut reads = Fetch::daemon_snapshot();
                // Jobs are the daemon's: a restore runs with no account open
                // (the entry page). The answer also says whether the daemon
                // has the full backup job API.
                if self.config.backup_api {
                    reads.push(Fetch::BackupJobs);
                }
                reads
            }
            Scope::Account(id) => {
                let mut reads = Fetch::account_snapshot(id);
                // Calls and call records come back with every snapshot;
                // the answer also says whether the daemon has the call API.
                if self.config.call_api {
                    reads.push(Fetch::Calls);
                }
                // Pending contact requests and the request setting; the
                // answer also says whether the daemon has requests.
                reads.push(Fetch::ContactRequests);
                // Jobs come back with every snapshot too; the answer also
                // says whether the daemon has the full backup job API.
                if self.config.backup_api {
                    reads.push(Fetch::BackupJobs);
                }
                reads
            }
        }
    }

    /// Whether the daemon has the call API, as its `call.list` answer
    /// says: the call methods are declared supported or not accordingly.
    fn calls_answered(&mut self, live: bool) {
        let found = capabilities::Found { calls: live, ..self.found };
        self.found_now(found);
    }

    /// Whether the daemon has contact requests, as its
    /// `contact.requests.list` answer says.
    fn requests_answered(&mut self, live: bool) {
        let found = capabilities::Found { requests_missing: !live, ..self.found };
        self.found_now(found);
    }

    fn found_now(&mut self, found: capabilities::Found) {
        if found != self.found {
            self.found = found;
            self.emit(vec![DomainEvent::CapabilitiesUpdated(capabilities::capabilities(found))]);
        }
    }

    /// Whether the daemon has the full backup job API, as its
    /// `account.backup.jobs` answer says.
    fn backup_answered(&mut self, live: bool) {
        let found = capabilities::Found { backup: live, ..self.found };
        self.found_now(found);
        self.jobs_found(live);
    }

    /// Attaches the scope: recovery for the generation observed now. A live
    /// connection is recovered at once; a dead one is opened first and the
    /// generation it gets is recovered by the check (or by `Attached`).
    fn snapshot(&mut self) {
        self.link = Link::Attaching;
        self.cancel_reads();
        if self.client.is_connected() {
            self.attach_live();
        } else {
            self.attach_pending = true;
            self.spawn_attach(true);
        }
    }

    /// The connection is open: a generation not yet recovered is recovered;
    /// one that already has been is not recovered again (that would be a
    /// second decision for it), its reads are only read again in its epoch.
    pub(super) fn attach_live(&mut self) {
        if self.gate.enabled() && self.client.connections_opened() <= self.recovered_generation {
            self.attach_pending = false;
            self.cancel_reads();
            self.batch_pending = 0;
            self.start_epoch_reads(false);
        } else {
            self.recover_connection();
        }
    }

    /// A resync on the connection already recovered: the snapshot is read
    /// again through the ordered pipeline in the open epoch. Nothing is
    /// cancelled (reads in flight on this connection stay valid, whatever
    /// context they were issued for), and the gate is not closed.
    pub(super) fn resync(&mut self) {
        if self.gate.enabled() && self.client.connections_opened() <= self.recovered_generation {
            self.start_epoch_reads(false);
        } else {
            self.recover_connection();
        }
    }

    /// Opens the request connection if it is dead; the answer only wakes
    /// the generation check (and finishes a snapshot's attach).
    pub(super) fn spawn_attach(&self, snapshot: bool) {
        let (client, internal) = (self.client.clone(), self.internal.clone());
        tokio::spawn(async move {
            let result = client.attach().await;
            let _ = internal.send(Internal::Attached { snapshot, result });
        });
    }

    /// The one recovery routine. It always takes the generation observed at
    /// its start: outstanding reads are cancelled, a new epoch asks the
    /// daemon for its schema, and the prerequisite reads run before the
    /// messages.
    pub(super) fn recover_connection(&mut self) {
        self.recovered_generation = self.client.connections_opened();
        #[cfg(any(test, feature = "test-support"))]
        self.recoveries.push(self.recovered_generation);
        self.attach_pending = false;
        self.cancel_reads();
        self.start_epoch_reads(true);
    }

    /// The generation check, run on entry to every handler and on every
    /// notification: a request connection newer than the last one recovered
    /// is recovered, whatever the state of any status or notification.
    pub(super) fn observe_connection(&mut self) {
        if !self.gate.enabled() || matches!(self.link, Link::Locked | Link::Lost) {
            return;
        }
        if self.client.connections_opened() > self.recovered_generation {
            self.recover_connection();
        }
    }

    /// A new epoch's reads: its status, the local account state at once, the
    /// other non-message reads as a batch, and the message-class reads held
    /// until the gate opens and that batch is in.
    fn start_epoch_reads(&mut self, new_epoch: bool) {
        if new_epoch {
            self.begin_gate_epoch();
        } else {
            // The same epoch reads again.
            if self.gate.enabled() && self.gate.verdict() == SchemaGate::Unresolved {
                self.spawn_schema_status(self.gate.epoch());
            }
        }
        let mut reads = self.reads_for_snapshot();
        // Local unlocked state opens the workspace before unrelated lists finish.
        if let Some(account) = self.scope.account().cloned() {
            reads.retain(|fetch| !matches!(fetch, Fetch::AccountStatus(_)));
            self.spawn_fetches(vec![Fetch::AccountStatus(account)]);
        }
        reads.extend(self.translator.recovery_reads());
        // Message-class reads wait for the schema gate and for the
        // prerequisite batch (held in its queue, in order, and read as a
        // batch of their own once both are done).
        // The barrier stands before any read is admitted, so this holds on
        // every path, including a retry on an already open gate.
        self.gate.set_prerequisites(true);
        reads.retain(|fetch| self.gate.admit(fetch, true) == Admit::Run);
        self.gate.set_prerequisites(!reads.is_empty());
        self.run_batch(reads);
        self.flush_gate();
    }

    /// Reads `reads` together; they are translated in this order.
    fn run_batch(&mut self, reads: Vec<Fetch>) {
        if reads.is_empty() {
            return;
        }
        let reserved = reads.clone();
        #[cfg(any(test, feature = "test-support"))]
        for fetch in &reads {
            if gate::is_message_read(fetch) && !self.gate.pipeline_clear() {
                self.guard_violations += 1;
            }
        }
        let stamp = self.stamp();
        self.batch_pending += 1;
        let client = self.client.clone();
        let account = self.account();
        let internal = self.internal.clone();
        let sent = self.listings_sent.clone();
        let revision = self.networking_revision;
        let task = tokio::spawn(async move {
            let mut set = JoinSet::new();
            for (index, fetch) in reads.into_iter().enumerate() {
                let (client, account, sent) = (client.clone(), account.clone(), sent.clone());
                set.spawn(async move {
                    (index, read(&client, fetch, account.as_deref(), &sent, revision).await)
                });
            }
            let mut results = Vec::new();
            while let Some(Ok(answer)) = set.join_next().await {
                results.push(answer);
            }
            results.sort_by_key(|(index, _)| *index);
            let results = results.into_iter().map(|(_, read)| read).collect();
            let _ = internal.send(Internal::Snapshot(stamp, results));
        });
        let handle = task.abort_handle();
        for fetch in reserved {
            self.read_recovery.running.insert(fetch, handle.clone());
        }
        self.snapshot_task = Some(handle);
    }

    /// Stops the push subscription (the stream took over, or the scope
    /// ends).
    pub(super) fn stop_push_subscription(&mut self) {
        if let Some(task) = self.push_task.take() {
            task.abort();
        }
    }

    pub(super) fn lost(&mut self) {
        self.cancel_reads();
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
        self.sends.clear();
        self.cancel_reads();
        self.gate_retry_at = None;
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

    pub(super) fn spawn_fetches(&mut self, fetches: Vec<Fetch>) {
        self.spawn_reads(fetches, true);
    }

    fn spawn_reads(&mut self, fetches: Vec<Fetch>, rearm: bool) {
        if matches!(self.link, Link::Locked | Link::Lost) {
            return;
        }
        // Duplicates within one trigger are one request. A later trigger is
        // different: its committed change may not be in the running answer.
        let mut seen = BTreeSet::new();
        for fetch in fetches {
            if !seen.insert(fetch.clone()) {
                continue;
            }
            // A message-class read waits for (or is dropped by) the schema gate.
            if self.gate.admit(&fetch, false) != Admit::Run {
                continue;
            }
            if self.read_recovery.running.contains_key(&fetch) {
                if rearm {
                    self.read_recovery.rerun.insert(fetch);
                }
                continue;
            }
            if !rearm && self.read_recovery.blocks_timer(&fetch) {
                continue;
            }
            if rearm {
                self.read_recovery.rearm(&fetch);
            }
            // Bound spawned reads as well as requests on the bus. Recovery of
            // many loaded tasks/surfaces queues ids, rather than futures.
            if self.read_recovery.running.len() >= MAX_READS {
                self.read_recovery.enqueue(fetch);
                continue;
            }
            self.read_recovery.unqueue(&fetch);
            #[cfg(any(test, feature = "test-support"))]
            if gate::is_message_read(&fetch) && !self.gate.pipeline_clear() {
                self.guard_violations += 1;
            }
            let (client, account) = (self.client.clone(), self.account());
            let (internal, sent) = (self.internal.clone(), self.listings_sent.clone());
            let key = fetch.clone();
            let revision = self.networking_revision;
            let stamp = self.stamp();
            let task = tokio::spawn(async move {
                let read = read(&client, fetch, account.as_deref(), &sent, revision).await;
                let _ = internal.send(Internal::FetchedAt { stamp, read });
            });
            self.read_recovery.running.insert(key, task.abort_handle());
        }
    }

    /// Merge direct refreshes with a stream-requested follow-up that may
    /// already have started in read_ended. Never queue a third read for it.
    fn rerun_fetch(&mut self, fetch: &Fetch) {
        if self.read_recovery.rerun.remove(fetch) && !self.read_recovery.running.contains_key(fetch)
        {
            self.spawn_fetches(vec![fetch.clone()]);
        }
    }

    /// A read's completion as if dispatched now (fixtures).
    #[cfg(test)]
    pub(super) fn fetched_now(&self, read: Read) -> Internal {
        Internal::FetchedAt { stamp: self.stamp(), read }
    }

    /// The stamp reads are dispatched under now.
    pub(super) fn stamp(&self) -> Stamp {
        Stamp { epoch: self.gate.epoch(), attempt: self.read_attempt }
    }

    fn internal(&mut self, internal: Internal) {
        self.observe_connection();
        // The one place a superseded read is rejected: before bookkeeping
        // (running handles, pending counts, the barrier), before any
        // translation, before an error can lock the scope.
        if internal.stamp().is_some_and(|stamp| stamp != self.stamp()) {
            return;
        }
        match internal {
            Internal::Snapshot(stamp, results) => self.on_snapshot(stamp.epoch, results),
            Internal::SchemaAnswer { stamp, generation, result } => {
                if !matches!(self.link, Link::Locked | Link::Lost) {
                    self.on_schema_status(stamp.epoch, generation, &result);
                }
            }
            Internal::Attached { snapshot, result } => {
                // The check above has recovered a new generation already
                // (clearing `attach_pending`); this only finishes an attach
                // that opened nothing new, or reports that it failed.
                if snapshot && self.attach_pending && self.link == Link::Attaching {
                    match result {
                        Ok(_) => self.attach_live(),
                        Err(_) => self.lost(),
                    }
                }
            }
            Internal::EpochStart => {
                self.pending_markers = self.pending_markers.saturating_sub(1);
            }
            Internal::SendDone { id, result } => self.send_done(&id, result),
            Internal::FetchedAt { read: (fetch, number, result), .. } => {
                self.read_recovery.running.remove(&fetch);
                if matches!(self.link, Link::Locked | Link::Lost) {
                    return;
                }
                // A completion queued before this epoch began belongs to an
                // older connection; a message-class one that arrives while
                // the gate is closed is not translated either (it is read
                // again once the gate opens).
                if self.pending_markers > 0 {
                    return;
                }
                if self.gate.enabled() && !self.gate.is_open() && gate::is_message_read(&fetch) {
                    self.gate.note_dropped();
                    self.read_ended(&fetch, true);
                    return;
                }
                if self.link == Link::Attaching && matches!(fetch, Fetch::AccountStatus(_)) {
                    self.link = Link::Live;
                    if result.is_ok() {
                        // This local reply proves the daemon is reachable before
                        // unrelated snapshot lists (including backup jobs) finish.
                        self.emit(vec![DomainEvent::SourceStatusChanged(SourceStatus::Connected)]);
                    }
                }
                let finished = last_page(&fetch, &result);
                if !self.outdated(&fetch, number, &result) {
                    self.on_read(&fetch, result);
                    if matches!(fetch, Fetch::Page(Paged::Conversations | Paged::Groups, _)) {
                        self.restore_pins();
                    }
                } else if matches!(fetch, Fetch::AccountStatus(_)) {
                    self.on_stale_status(&fetch, result);
                }
                self.read_ended(&fetch, finished);
                self.rerun_fetch(&fetch);
            }
            Internal::EffectDone { method, body, result, refresh } => {
                self.on_effect_done(&method, &body, result, refresh);
            }
            Internal::ReceiptDone { key, sent } => {
                if let Some(receipts) = &mut self.receipts {
                    receipts.answered(&key, sent, Instant::now());
                }
            }
            Internal::Custody(outcome) => self.on_custody(outcome),
        }
        if !matches!(self.link, Link::Locked | Link::Lost) {
            let slots = MAX_READS.saturating_sub(self.read_recovery.running.len());
            let queued: Vec<_> =
                (0..slots).filter_map(|_| self.read_recovery.pop_queued()).collect();
            self.spawn_reads(queued, false);
        }
        self.send_receipts();
    }

    /// Queues the received direct messages listed since the last call and
    /// sends the delivery receipts that are due, each on its own: nothing
    /// waits for them, and a failed one is retried later (see
    /// [`crate::receipts`]).
    fn send_receipts(&mut self) {
        let listed = self.translator.take_delivered();
        let (Some(receipts), Some(account)) = (&mut self.receipts, self.scope.account()) else {
            return;
        };
        let now = Instant::now();
        receipts.observe(listed, now);
        if self.link != Link::Live || !self.gate.is_open() {
            return;
        }
        for delivered in receipts.due(now) {
            let request = delivered.request(account.as_str());
            let (client, internal, key) =
                (self.client.clone(), self.internal.clone(), delivered.key());
            tokio::spawn(async move {
                let sent = client.call(&request).await.is_ok();
                let _ = internal.send(Internal::ReceiptDone { key, sent });
            });
        }
    }

    fn on_snapshot(&mut self, epoch: u64, mut results: Vec<Read>) {
        for (fetch, _, _) in &results {
            self.read_recovery.running.remove(fetch);
        }
        // A batch of an older epoch is never translated: its status (and
        // every other answer) belongs to a connection that is gone.
        if epoch != self.gate.epoch() || self.pending_markers > 0 {
            return;
        }
        self.snapshot_task = None;
        self.batch_pending = self.batch_pending.saturating_sub(1);
        if matches!(self.link, Link::Locked | Link::Lost) {
            return;
        }
        if self.gate.enabled() && !self.gate.is_open() {
            let before = results.len();
            results.retain(|(fetch, _, _)| !gate::is_message_read(fetch));
            if results.len() != before {
                self.gate.note_dropped();
            }
        }
        let unanswered = |r: &Result<Value, BusError>| {
            r.as_ref().is_err_and(|e| e.is_connection_lost() || *e == BusError::Timeout)
        };
        // A daemon that accepts the connection but answers none of the reads
        // in time is treated as gone (and retried), not as an empty one.
        if results.iter().any(|(_, _, r)| r.as_ref().is_err_and(BusError::is_connection_lost))
            || (self.scope.account().is_none() && results.iter().all(|(_, _, r)| unanswered(r)))
        {
            self.lost();
            return;
        }
        self.link = Link::Live;
        for (fetch, number, result) in results {
            let finished = last_page(&fetch, &result);
            if !self.outdated(&fetch, number, &result) {
                self.on_read(&fetch, result);
            } else if matches!(fetch, Fetch::AccountStatus(_)) {
                self.on_stale_status(&fetch, result);
            }
            self.read_ended(&fetch, finished);
            self.rerun_fetch(&fetch);
        }
        // Every list is known now, so pins come back in their saved order.
        self.restore_pins();
        if self.link == Link::Live {
            self.live_after_snapshot();
        }
        if self.batch_pending == 0 {
            self.gate.set_prerequisites(false);
            self.flush_gate();
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
        if matches!(fetch, Fetch::AccountStatus(_)) && result.is_ok() {
            return number.is_some_and(|revision| revision < self.networking_revision);
        }
        let (Fetch::Page(paged, None), Some(number), Ok(_)) = (fetch, number, result) else {
            return false;
        };
        if self.listing_applied.get(paged).is_some_and(|applied| *applied > number) {
            return true;
        }
        self.listing_applied.insert(*paged, number);
        false
    }

    /// A late status still establishes local readiness and identity. Only its
    /// networking half is superseded by the event applied after dispatch.
    fn on_stale_status(&mut self, fetch: &Fetch, result: Result<Value, BusError>) {
        let local = result.map(|mut value| {
            if let Some(body) = value.as_object_mut() {
                for field in [
                    "networking",
                    "networking_snapshot",
                    "session",
                    "networking_reason",
                    "node_binding",
                    "authority_blocked",
                ] {
                    body.remove(field);
                }
            }
            value
        });
        self.on_read(fetch, local);
    }

    /// Translates one read; returns whether it produced events.
    fn on_read(&mut self, fetch: &Fetch, result: Result<Value, BusError>) -> bool {
        // An account made by an older development build cannot be opened:
        // nothing is retried and nothing is written to it.
        if let Err(BusError::Daemon { code, .. }) = &result
            && code == "SchemaResetRequired"
            && self.gate.enabled()
        {
            self.gate.reset_required();
            self.gate_changed();
            return false;
        }
        if let Err(error) = &result
            // Network receives retain their refusal handling; bounded local-read
            // recovery must not turn a refused pull into a failed user effect.
            && !matches!(fetch, Fetch::GroupReceive { .. })
            && !error.is_connection_lost()
            && !error.is_unknown_method()
            && !(matches!(fetch, Fetch::BackupJobs)
                && (!error.is_retryable()
                    || matches!(error, BusError::Daemon { code, .. } if code == "CapabilityDenied")))
            && !(matches!(fetch, Fetch::AccountStatus(_))
                && matches!(error, BusError::Daemon { message, .. } if message.contains("account not open")))
        {
            if !self.read_recovery.failed(fetch, Instant::now()) {
                return false;
            }
            if matches!(fetch, Fetch::ContactRequests) {
                let events = self.translator.requests_read_failed(error);
                return self.emit(events);
            }
            let failure = ramflux_desktop_core::EffectFailure {
                method: fetch.request(self.account().as_deref()).method,
                code: match error {
                    BusError::Timeout => ramflux_desktop_core::FailureCode::Timeout,
                    BusError::Transport(_) => ramflux_desktop_core::FailureCode::Transport,
                    BusError::Daemon { code, .. } => {
                        ramflux_desktop_core::FailureCode::Daemon(code.clone())
                    }
                },
                message: error.message(),
            };
            return self.emit(vec![DomainEvent::EffectFailed(failure)]);
        }
        self.read_recovery.succeeded(fetch);
        self.on_read_inner(fetch, result)
    }

    fn on_read_inner(&mut self, fetch: &Fetch, result: Result<Value, BusError>) -> bool {
        if matches!(fetch, Fetch::Calls | Fetch::ContactRequests) {
            let calls = matches!(fetch, Fetch::Calls);
            // Only a method the daemon does not know says the API is
            // missing; any other failure leaves what was found.
            let live = match &result {
                Ok(_) => Some(true),
                Err(error) if error.is_unknown_method() => Some(false),
                Err(_) => None,
            };
            match (live, calls) {
                (Some(live), true) => self.calls_answered(live),
                (Some(live), false) => self.requests_answered(live),
                (None, _) => {}
            }
        }
        if matches!(fetch, Fetch::DaemonStatus)
            && let Ok(status) = &result
        {
            // Only the daemon's own word offers creation with a phrase.
            let offered = status["born_v3_creation"].as_bool() == Some(true);
            self.found_now(capabilities::Found { born_v3: offered, ..self.found });
            // Once this epoch's status is known, a later one that differs
            // re-evaluates the gate (never before: an answer of an older
            // connection could otherwise open it).
            if self.gate.reevaluate(status) {
                self.gate_changed();
            }
        }
        if matches!(fetch, Fetch::BackupJobs) {
            // Only a method the daemon does not know says the API is
            // missing (a daemon refusing backups to a non-human account
            // too); any other refusal is a read failure.
            match &result {
                Ok(_) => self.backup_answered(true),
                Err(error @ BusError::Daemon { code, .. })
                    if error.is_unknown_method() || code == "CapabilityDenied" =>
                {
                    self.backup_answered(false);
                }
                Err(_) => {}
            }
            // A refusal that trying again will not change is the answer: the
            // listing is not coming, and nobody should wait for it. (A lost
            // connection or a missed deadline is retried, not given up.)
            if result.as_ref().is_err_and(|error| !error.is_retryable()) {
                let given_up = self.translator.backup_jobs_given_up();
                self.emit(given_up);
            }
        }
        match result {
            Ok(value) => {
                let translated = self.translator.fetched(fetch, &value, now());
                self.spawn_fetches(translated.follow);
                let emitted = self.emit(translated.events);
                // A surface answer to a task step, now with its receipt.
                for body in self.translator.take_task_resolutions() {
                    self.effect(ramflux_desktop_core::Effect::Call {
                        method: ramflux_desktop_core::methods::TASK_RESOLVE.to_owned(),
                        body,
                    });
                }
                emitted
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
            // unreachable) changes nothing; a later event or action can retry.
            Err(_) => false,
        }
    }

    fn on_pushed(&mut self, pushed: Pushed) {
        match pushed {
            Pushed::Opened => {
                self.refresh_daemon();
                if let Some(id) = self.scope.account().cloned() {
                    self.spawn_fetches(vec![Fetch::AccountStatus(id)]);
                }
            }
            Pushed::Event { topic, body } if topic == "account.networking.changed" => {
                self.networking_event(&body);
            }
            Pushed::Event { topic, body }
                if crate::translate::contact_requests::TOPICS.contains(&topic.as_str()) =>
            {
                let translated = self.translator.contact_request_event(&topic, &body);
                self.spawn_fetches(translated.follow);
                self.emit(translated.events);
            }
            Pushed::Event { topic, body } => {
                let events = self.translator.pushed(&topic, &body, now());
                self.emit(events);
            }
            Pushed::Lost(_) => {
                self.push_task = None;
            }
        }
    }

    fn cancel_reads(&mut self) {
        // Completions already queued by the reads being cancelled belong to
        // the attempt that ends here.
        self.read_attempt += 1;
        self.read_recovery.cancel();
        self.sync.clear_reads();
        if let Some(task) = self.snapshot_task.take() {
            task.abort();
        }
    }

    /// Reads a call's media session for the engine. Its secrets go
    /// straight to the source that asked: never to the translator, an
    /// event or a log.
    fn call_media(
        &self,
        call_id: ramflux_desktop_core::CallId,
        reply: crate::completion::CompletionSender<super::MediaAnswer>,
    ) {
        let account = match self.account() {
            Some(account) if self.found.calls => account,
            _ => {
                let _ = reply.send((call_id, Err(capabilities::reason::CALLS.to_owned())));
                return;
            }
        };
        let client = self.client.clone();
        tokio::spawn(async move {
            let request = crate::plan::call::media_session(&account, call_id.as_str());
            let answer = match client.call(&request).await {
                Ok(answer) => crate::translate::calls::call_media(&call_id, answer),
                Err(error) => Err(error.message()),
            };
            let _ = reply.send((call_id, answer));
        });
    }

    fn command(&mut self, command: ScopeCommand) {
        self.observe_connection();
        match command {
            ScopeCommand::Focus(context) => {
                if let Some(context) = &context
                    && self.link == Link::Live
                {
                    let reads = self.translator.context_reads(context);
                    for fetch in &reads {
                        self.read_recovery.prioritize(fetch);
                    }
                    self.request_reads(reads);
                }
                self.translator.set_focus(context);
            }
            ScopeCommand::Effect(effect) => self.effect(*effect),
            ScopeCommand::Bodies(moments) => {
                let reads = self.translator.request_bodies(&moments);
                self.spawn_fetches(reads);
            }
            #[cfg(feature = "test-support")]
            ScopeCommand::MediaFailed(object_id, failure) => {
                let translated = crate::translate::v1::candidate::media_failed(
                    &mut self.translator,
                    &object_id,
                    &failure,
                );
                self.emit(translated.events);
            }
            #[cfg(feature = "test-support")]
            ScopeCommand::Generated(updates) => {
                if !self.gate.is_open() {
                    return;
                }
                let translated =
                    crate::translate::v1::candidate::apply(&mut self.translator, &updates);
                self.emit(translated.events);
            }
            #[cfg(feature = "test-support")]
            ScopeCommand::RecheckSchema => self.recheck_schema(),
            #[cfg(feature = "test-support")]
            ScopeCommand::GateProbe(reply) => {
                let _ = reply.send(GateProbe {
                    epoch: self.gate.epoch(),
                    gate: self.gate.verdict(),
                    queued: self.gate.queued(),
                    plaintext: self.translator.plaintext_len(),
                    receipts: self.receipts.as_ref().map_or(0, Receipts::pending_len),
                });
            }
            ScopeCommand::CallMedia(call_id, reply) => self.call_media(call_id, reply),
            ScopeCommand::AccountsLive(live) => self.accounts_live(live),
            ScopeCommand::RefreshDaemon => self.refresh_daemon(),
            ScopeCommand::ConnectionStatus(status) => self.connection_status(status),
        }
    }
}

#[cfg(test)]
#[path = "requests_retry_tests.rs"]
mod requests_retry_tests;

#[cfg(test)]
#[path = "review_retry_tests.rs"]
mod review_retry_tests;

#[cfg(test)]
#[path = "networking_tests.rs"]
mod networking_tests;

#[cfg(test)]
#[path = "idle_tests.rs"]
mod idle_tests;

#[cfg(test)]
#[path = "gate_tests.rs"]
mod gate_tests;
