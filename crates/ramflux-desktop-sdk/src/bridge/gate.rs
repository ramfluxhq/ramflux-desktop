// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The schema gate (S10c D1): a connection-scoped state machine that decides
//! whether an account scope may send message-class requests.
//!
//! The daemon's `schema_generation` (from `daemon.status`) and this build's
//! [`SchemaSupport`] give the verdict:
//!
//! | status | Legacy | V1 |
//! |---|---|---|
//! | in flight (new epoch) | held | held |
//! | error or timeout | held, retried | held, retried |
//! | field absent, or 0 | open | older daemon |
//! | 1 | update the app | open |
//! | anything else | unsupported | unsupported |
//!
//! Every connect or reconnect starts a new epoch: the approval is revoked,
//! the old queue is dropped, and an answer carrying an older epoch can never
//! open the gate. While held, message-class reads wait in a bounded queue
//! (released in order on open); while blocked they are dropped, and a flag
//! says they must be read again once the gate opens.
//!
//! Message-class requests are the conversation, message, history, surface,
//! task, call and object reads, message sends, edits, reactions, deletes,
//! receipts, typing and SelfSync. Daemon status, the account and node lists,
//! diagnostics, backup jobs and custody ceremonies stay open.

use ramflux_desktop_core::SchemaGate;
use serde_json::Value;

use crate::config::SchemaSupport;
use crate::plan::fetch::{Fetch, Paged};

/// Reads held per account while the answer is awaited.
pub(super) const MAX_QUEUE: usize = 512;

/// What a status answer said.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Status {
    /// The answer is awaited.
    Unresolved,
    /// The read failed or timed out in this epoch.
    Failed,
    /// A successful answer without the field (a daemon older than p00).
    Absent,
    /// A successful answer with an integer generation.
    Known(i64),
    /// A successful answer whose generation is not an integer.
    Unknown,
}

/// Reads a successful `daemon.status` answer.
pub(super) fn classify(answer: &Value) -> Status {
    let Some(object) = answer.as_object() else { return Status::Unknown };
    match object.get("schema_generation") {
        None => Status::Absent,
        Some(value) => value.as_i64().map_or(Status::Unknown, Status::Known),
    }
}

/// The verdict for a status under a support level.
pub(super) fn verdict(status: Status, support: SchemaSupport) -> SchemaGate {
    match status {
        Status::Unresolved => SchemaGate::Unresolved,
        Status::Failed => SchemaGate::Failed,
        Status::Unknown => SchemaGate::Unsupported,
        Status::Absent | Status::Known(0) => match support {
            SchemaSupport::Legacy => SchemaGate::Open,
            #[cfg(feature = "test-support")]
            SchemaSupport::V1Candidate => SchemaGate::OlderDaemon,
        },
        Status::Known(1) => match support {
            SchemaSupport::Legacy => SchemaGate::UpdateApp,
            #[cfg(feature = "test-support")]
            SchemaSupport::V1Candidate => SchemaGate::Open,
        },
        Status::Known(_) => SchemaGate::Unsupported,
    }
}

/// Whether a read is message-class.
pub(super) fn is_message_read(fetch: &Fetch) -> bool {
    match fetch {
        Fetch::Page(paged, _) => {
            matches!(paged, Paged::Conversations | Paged::Groups | Paged::Objects)
        }
        Fetch::Messages(_)
        | Fetch::Surfaces(..)
        | Fetch::Surface(_)
        | Fetch::Tasks(..)
        | Fetch::Task(..)
        | Fetch::Calls
        | Fetch::GroupReceive { .. } => true,
        _ => false,
    }
}

pub(super) use crate::bus::approval::is_message_method;

/// What the gate says about a read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Admit {
    /// Send it now.
    Run,
    /// It waits in the queue.
    Held,
    /// It is dropped (and read again once the gate opens).
    Dropped,
}

/// One account scope's gate.
#[derive(Debug)]
pub(super) struct Gate {
    enabled: bool,
    support: SchemaSupport,
    epoch: u64,
    status: Status,
    generation: Option<i64>,
    reset: bool,
    /// Message-class reads waiting for the answer, with whether each belongs
    /// to the snapshot batch (read in order) or is a single read.
    queue: Vec<(Fetch, bool)>,
    /// Reads were dropped while blocked: read the messages again on open.
    dropped: bool,
    /// The prerequisite reads (identities, devices, ...) of this epoch are
    /// still in flight: message-class reads wait behind them, whichever
    /// path asks for them.
    prerequisites: bool,
}

impl Gate {
    /// A gate; a disabled gate (the daemon scope) is always open.
    pub(super) fn new(enabled: bool, support: SchemaSupport) -> Self {
        Self {
            enabled,
            support,
            epoch: 0,
            status: Status::Unresolved,
            generation: None,
            reset: false,
            queue: Vec::new(),
            dropped: false,
            prerequisites: false,
        }
    }

    pub(super) fn enabled(&self) -> bool {
        self.enabled
    }

    pub(super) fn epoch(&self) -> u64 {
        self.epoch
    }

    /// The current verdict.
    pub(super) fn verdict(&self) -> SchemaGate {
        if !self.enabled {
            // The daemon scope has no schema to check.
            SchemaGate::Open
        } else if self.reset {
            SchemaGate::ResetRequired
        } else {
            verdict(self.status, self.support)
        }
    }

    /// The generation the daemon last reported in this epoch.
    pub(super) fn generation(&self) -> Option<i64> {
        self.generation
    }

    pub(super) fn is_open(&self) -> bool {
        self.verdict().is_open()
    }

    /// A new connection: the approval is revoked and the old queue dropped.
    pub(super) fn begin_epoch(&mut self) -> u64 {
        self.epoch += 1;
        self.status = Status::Unresolved;
        self.generation = None;
        self.reset = false;
        self.queue.clear();
        self.dropped = false;
        self.prerequisites = false;
        self.epoch
    }

    /// Marks the epoch's prerequisite reads as in flight (or done).
    pub(super) fn set_prerequisites(&mut self, pending: bool) {
        self.prerequisites = pending;
    }

    /// Whether message-class reads still wait for the prerequisite reads.
    #[cfg(test)]
    pub(super) fn prerequisites_pending(&self) -> bool {
        self.prerequisites
    }

    /// Whether a message-class read may be sent right now.
    pub(super) fn pipeline_clear(&self) -> bool {
        !self.enabled || (self.is_open() && !self.prerequisites)
    }

    /// A status read of `epoch` came back. An answer of another epoch is
    /// ignored; returns whether the answer counted.
    pub(super) fn resolve(&mut self, epoch: u64, answer: Result<&Value, ()>) -> bool {
        if !self.enabled || self.epoch == 0 || epoch != self.epoch {
            return false;
        }
        match answer {
            Ok(value) => {
                self.status = classify(value);
                self.generation = match self.status {
                    Status::Known(n) => Some(n),
                    _ => None,
                };
            }
            Err(()) => {
                self.status = Status::Failed;
                self.generation = None;
            }
        }
        true
    }

    /// A later `daemon.status` answer of an epoch whose status is known:
    /// returns whether it was taken.
    pub(super) fn reevaluate(&mut self, answer: &Value) -> bool {
        if !self.enabled
            || self.epoch == 0
            || matches!(self.status, Status::Unresolved | Status::Failed)
        {
            return false;
        }
        // A later answer can close the gate or confirm it; reopening needs
        // a status read that is checked against its connection.
        let status = classify(answer);
        if !self.is_open() && verdict(status, self.support).is_open() {
            return false;
        }
        self.status = status;
        self.generation = match self.status {
            Status::Known(n) => Some(n),
            _ => None,
        };
        true
    }

    /// A message-class result was dropped while closed: read it again once
    /// the gate opens.
    pub(super) fn note_dropped(&mut self) {
        self.dropped = true;
    }

    /// A status read is being repeated in this epoch.
    pub(super) fn retry(&mut self) {
        if matches!(self.status, Status::Failed) {
            self.status = Status::Unresolved;
        }
    }

    /// The daemon refused with `SchemaResetRequired`.
    pub(super) fn reset_required(&mut self) {
        if self.enabled {
            self.reset = true;
            self.queue.clear();
        }
    }

    /// Decides what happens to a read; a held read joins the queue.
    pub(super) fn admit(&mut self, fetch: &Fetch, snapshot: bool) -> Admit {
        if !self.enabled || !is_message_read(fetch) || self.pipeline_clear() {
            return Admit::Run;
        }
        match self.verdict() {
            // Open but behind the prerequisites: held like an unresolved one.
            SchemaGate::Open | SchemaGate::Unresolved | SchemaGate::Failed => {
                if self.queue.iter().any(|(held, _)| held == fetch) {
                    return Admit::Held;
                }
                if self.queue.len() >= MAX_QUEUE {
                    self.dropped = true;
                    return Admit::Dropped;
                }
                self.queue.push((fetch.clone(), snapshot));
                Admit::Held
            }
            _ => {
                self.dropped = true;
                Admit::Dropped
            }
        }
    }

    /// The queue, in order, once the gate opened; and whether reads were
    /// dropped meanwhile (then the snapshot's message reads are due again).
    pub(super) fn release(&mut self) -> (Vec<(Fetch, bool)>, bool) {
        (std::mem::take(&mut self.queue), std::mem::take(&mut self.dropped))
    }

    /// Reads waiting.
    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn queued(&self) -> usize {
        self.queue.len()
    }

    /// The waiting reads, in the order they will be sent.
    #[cfg(test)]
    pub(super) fn queued_reads(&self) -> Vec<Fetch> {
        self.queue.iter().map(|(fetch, _)| fetch.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use ramflux_desktop_core::ContextId;
    use serde_json::json;

    use super::*;

    #[test]
    fn classify_reads_only_integers() {
        assert_eq!(classify(&json!({})), Status::Absent);
        assert_eq!(classify(&json!({ "schema_generation": 0 })), Status::Known(0));
        assert_eq!(classify(&json!({ "schema_generation": 1 })), Status::Known(1));
        assert_eq!(classify(&json!({ "schema_generation": 2 })), Status::Known(2));
        assert_eq!(classify(&json!({ "schema_generation": -1 })), Status::Known(-1));
        for odd in [json!("1"), json!(1.5), json!(null), json!(true), json!([1])] {
            assert_eq!(classify(&json!({ "schema_generation": odd })), Status::Unknown, "{odd}");
        }
        assert_eq!(classify(&json!("daemon")), Status::Unknown);
    }

    #[test]
    fn the_table_for_the_legacy_build() {
        let v = |s| verdict(s, SchemaSupport::Legacy);
        assert_eq!(v(Status::Unresolved), SchemaGate::Unresolved);
        assert_eq!(v(Status::Failed), SchemaGate::Failed);
        assert_eq!(v(Status::Absent), SchemaGate::Open);
        assert_eq!(v(Status::Known(0)), SchemaGate::Open);
        assert_eq!(v(Status::Known(1)), SchemaGate::UpdateApp);
        assert_eq!(v(Status::Known(2)), SchemaGate::Unsupported);
        assert_eq!(v(Status::Known(-1)), SchemaGate::Unsupported);
        assert_eq!(v(Status::Unknown), SchemaGate::Unsupported);
    }

    #[test]
    fn messages_are_classified() {
        assert!(is_message_read(&Fetch::Messages(ContextId::new("c"))));
        assert!(is_message_read(&Fetch::first(Paged::Conversations)));
        assert!(is_message_read(&Fetch::Calls));
        assert!(!is_message_read(&Fetch::DaemonStatus));
        assert!(!is_message_read(&Fetch::AccountList));
        assert!(!is_message_read(&Fetch::BackupJobs));
        assert!(!is_message_read(&Fetch::first(Paged::Identities)));
        assert!(is_message_method("message.submit") && is_message_method("surface.action"));
        assert!(!is_message_method("account.backup.create") && !is_message_method("daemon.status"));
    }

    #[test]
    fn an_old_epoch_never_opens_the_gate() {
        let mut gate = Gate::new(true, SchemaSupport::Legacy);
        let first = gate.begin_epoch();
        assert!(gate.resolve(first, Ok(&json!({ "schema_generation": 0 }))));
        assert!(gate.is_open());
        let second = gate.begin_epoch();
        assert!(!gate.is_open(), "a new epoch revokes the approval");
        assert!(!gate.resolve(first, Ok(&json!({ "schema_generation": 0 }))));
        assert_eq!(gate.verdict(), SchemaGate::Unresolved);
        assert!(gate.resolve(second, Ok(&json!({ "schema_generation": 0 }))));
        assert!(gate.is_open());
    }

    #[test]
    fn held_reads_wait_in_a_bounded_queue_and_blocked_reads_are_dropped() {
        let mut gate = Gate::new(true, SchemaSupport::Legacy);
        gate.begin_epoch();
        let read = Fetch::Messages(ContextId::new("c"));
        assert_eq!(gate.admit(&Fetch::AccountList, false), Admit::Run);
        assert_eq!(gate.admit(&read, false), Admit::Held);
        assert_eq!(gate.admit(&read, false), Admit::Held, "one entry per read");
        assert_eq!(gate.queued(), 1);
        for n in 0..(MAX_QUEUE + 4) {
            gate.admit(&Fetch::Messages(ContextId::new(format!("c{n}"))), false);
        }
        assert_eq!(gate.queued(), MAX_QUEUE);
        gate.resolve(gate.epoch(), Ok(&json!({ "schema_generation": 7 })));
        assert_eq!(gate.admit(&read, false), Admit::Dropped);
        let (queue, dropped) = gate.release();
        assert_eq!(queue.len(), MAX_QUEUE);
        assert!(dropped);
    }

    #[test]
    fn a_disabled_gate_is_open() {
        let mut gate = Gate::new(false, SchemaSupport::Legacy);
        gate.begin_epoch();
        assert!(gate.is_open());
        assert_eq!(gate.admit(&Fetch::Calls, false), Admit::Run);
    }
}
