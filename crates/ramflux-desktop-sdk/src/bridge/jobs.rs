// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! How a scope follows the daemon's backup and restore jobs (see
//! [`crate::bus::jobs`]).
//!
//! Once a snapshot found the job API (`account.backup.jobs` answered), the
//! scope opens its job link: the attended connection with the daemon-level
//! job stream, on which the job requests are sent too. Each time the stream
//! opens the jobs are read again (what changed while it was closed; after a
//! daemon restart, which jobs went with the daemon). A stream that fell
//! behind is opened again at once from its cursor; a lost one after the
//! reconnect back-off; a refused one only after the next snapshot finds the
//! API again. The daemon scope keeps its link only while no account scope
//! is attached: an account's link follows the jobs then.

use tokio::time::Instant;

use super::scope::{Link, Task};
use crate::bus::jobs::JobStreamed;
use crate::bus::stream::StreamEnd;
use crate::plan::fetch::Fetch;

/// Where the job link is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Jobs {
    /// Not open (and not being opened).
    Closed,
    /// Being opened; `epoch` is the link's close count when the open was
    /// asked for (see [`crate::bus::jobs::JobLink::epoch`]).
    Opening { epoch: u64 },
    /// The stream is open.
    Open,
}

impl Task {
    /// Whether this scope leaves the jobs to another: the daemon scope
    /// while an account scope is attached (that one follows them on its
    /// own link, so one attended connection and one set of job reads).
    pub(super) fn jobs_yield(&self) -> bool {
        self.accounts_live && self.scope.account().is_none()
    }

    /// Daemon scope: an account scope attached (`live`) or the last one
    /// detached. The link closes while one is attached and opens again
    /// once none is.
    pub(super) fn accounts_live(&mut self, live: bool) {
        self.accounts_live = live;
        if self.scope.account().is_some() {
            return;
        }
        if live {
            self.jobs_open_at = None;
            if self.jobs != Jobs::Closed {
                self.jobs = Jobs::Closed;
                self.job_link.close_soon();
            }
        } else if self.found.backup && self.jobs == Jobs::Closed {
            self.jobs_open_at = Some(Instant::now());
        }
    }

    /// The daemon has (or lacks) the job API: open the link if it is closed.
    pub(super) fn jobs_found(&mut self, live: bool) {
        if live && self.jobs == Jobs::Closed && self.jobs_open_at.is_none() && !self.jobs_yield() {
            self.jobs_open_at = Some(Instant::now());
        }
        if !live {
            self.jobs_open_at = None;
        }
    }

    /// Opens the job link (when due).
    pub(super) fn open_jobs(&mut self) {
        self.jobs_open_at = None;
        if self.jobs != Jobs::Closed || !self.found.backup || self.jobs_yield() {
            return;
        }
        self.jobs = Jobs::Opening { epoch: self.job_link.epoch() };
        let link = self.job_link.clone();
        tokio::spawn(async move {
            // Its outcome arrives as `Opened` or `Ended`.
            let _ = link.ensure().await;
        });
    }

    pub(super) fn on_jobs(&mut self, streamed: JobStreamed) {
        match streamed {
            // A connection closed since it opened. When the open this scope
            // is waiting for was the one (a job request lost its connection
            // meanwhile): nothing is open, and no other notice follows, so
            // the link is open again. A notice of an earlier open (an
            // account attached and detached during it) is left alone: the
            // open this scope asked for after the detach follows.
            JobStreamed::Opened { epoch, .. } if epoch != self.job_link.epoch() => {
                if matches!(self.jobs, Jobs::Opening { epoch: asked } if epoch >= asked) {
                    self.jobs = Jobs::Closed;
                    self.job_link.close_soon();
                    if self.found.backup && !self.jobs_yield() {
                        self.jobs_open_at = Some(Instant::now());
                    }
                }
            }
            JobStreamed::Opened { .. } if self.jobs_yield() => {
                // An account scope attached while this link was opening:
                // that one follows the jobs now. The link that just opened
                // is closed (the attach already asked for it before the
                // open finished), and this scope is closed, not Open, so
                // the last account detaching opens it again.
                self.jobs = Jobs::Closed;
                self.jobs_open_at = None;
                self.job_link.close_soon();
            }
            JobStreamed::Opened { resync, .. } => {
                self.jobs = Jobs::Open;
                if resync {
                    // The daemon holds nothing up to the cursor followed: it
                    // restarted, and the jobs it ran are gone with it. The
                    // listing below settles those reported as running.
                    self.translator.backup_stream_resynced();
                }
                // Whatever changed while the stream was not open.
                self.request_reads(vec![Fetch::BackupJobs]);
            }
            JobStreamed::Row { body, .. } => {
                let translated = self.translator.backup_job_row(&body);
                self.request_reads(translated.follow);
                self.emit(translated.events);
            }
            JobStreamed::Ended(end) => {
                self.jobs = Jobs::Closed;
                self.job_link.close_soon();
                if self.link != Link::Live || !self.found.backup || self.jobs_yield() {
                    return;
                }
                let now = Instant::now();
                self.jobs_open_at = match end {
                    StreamEnd::Lagged { .. } | StreamEnd::ResyncRequired | StreamEnd::NotOpen => {
                        Some(now)
                    }
                    StreamEnd::Lost(_) => Some(now + self.config.reconnect_backoff),
                    StreamEnd::Unsupported(_) => None,
                };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc as std_mpsc;

    use super::*;
    use crate::bridge::scope::build;
    use crate::config::SdkConfig;
    use crate::translate::Scope;

    /// A daemon scope that found the job API and is live, its link being
    /// opened (asked for at the link's current close count). Nothing
    /// connects: the notices are fed by hand, in the order under test.
    fn opening_daemon_scope() -> (Task, std_mpsc::Receiver<crate::bridge::Delivery>) {
        let (outbox, deliveries) = std_mpsc::channel();
        let (mut task, _inbox) =
            build(SdkConfig::new("/nonexistent/rfd.sock"), Scope::Daemon, outbox, ceremonies());
        task.link = Link::Live;
        task.found.backup = true;
        task.jobs = Jobs::Opening { epoch: task.job_link.epoch() };
        (task, deliveries)
    }

    fn ceremonies() -> crate::bridge::scope::Ceremonies {
        let (phrases, _) = std_mpsc::channel();
        crate::bridge::scope::Ceremonies { phrases, audit: std::sync::Arc::default() }
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime")
    }

    #[test]
    fn a_notice_from_before_an_attach_and_detach_never_marks_the_link_open() {
        runtime().block_on(async {
            let (mut task, _deliveries) = opening_daemon_scope();
            // An account attaches and detaches while the open is in flight;
            // the scope handled both before the open's notice arrives.
            task.accounts_live(true);
            task.accounts_live(false);
            assert!(task.jobs_open_at.is_some(), "the last detach asks for a new open");
            task.on_jobs(JobStreamed::Opened { resync: false, epoch: 0 });
            assert_eq!(task.jobs, Jobs::Closed, "the notice is about a connection that closed");
            assert!(
                task.jobs_open_at.is_some(),
                "and the open asked for after the detach stays due"
            );
        });
    }

    #[test]
    fn a_discarded_notice_of_the_open_being_waited_for_schedules_a_reopen() {
        runtime().block_on(async {
            let (mut task, _deliveries) = opening_daemon_scope();
            // A job request lost its connection: the link was closed from
            // under the open in flight.
            task.job_link.close_soon();
            assert_eq!(task.jobs_open_at, None);
            task.on_jobs(JobStreamed::Opened { resync: false, epoch: 0 });
            assert_eq!(task.jobs, Jobs::Closed);
            assert!(task.jobs_open_at.is_some(), "nothing else reports that open: reopen");
        });
    }
}
