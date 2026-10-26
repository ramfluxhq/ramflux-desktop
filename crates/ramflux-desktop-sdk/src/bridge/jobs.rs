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
//! API again.

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
    /// Being opened.
    Opening,
    /// The stream is open.
    Open,
}

impl Task {
    /// The daemon has (or lacks) the job API: open the link if it is closed.
    pub(super) fn jobs_found(&mut self, live: bool) {
        if live && self.jobs == Jobs::Closed && self.jobs_open_at.is_none() {
            self.jobs_open_at = Some(Instant::now());
        }
        if !live {
            self.jobs_open_at = None;
        }
    }

    /// Opens the job link (when due).
    pub(super) fn open_jobs(&mut self) {
        self.jobs_open_at = None;
        if self.jobs != Jobs::Closed || !self.found.backup {
            return;
        }
        self.jobs = Jobs::Opening;
        let link = self.job_link.clone();
        tokio::spawn(async move {
            // Its outcome arrives as `Opened` or `Ended`.
            let _ = link.ensure().await;
        });
    }

    pub(super) fn on_jobs(&mut self, streamed: JobStreamed) {
        match streamed {
            JobStreamed::Opened { resync } => {
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
                let link = self.job_link.clone();
                tokio::spawn(async move { link.close().await });
                if self.link != Link::Live || !self.found.backup {
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
