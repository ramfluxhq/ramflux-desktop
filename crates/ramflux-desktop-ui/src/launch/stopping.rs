// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! How a daemon the desktop started is stopped: gracefully first, then by
//! signal, then by force, each step bounded.
//!
//! The launcher asks the daemon to stop over the local bus (`daemon.stop`:
//! it stops accepting, drains, closes its databases and removes its socket)
//! and polls. The process is sent a terminate signal when the daemon refuses
//! or cannot be asked, or when it has not exited after
//! [`GRACEFUL_POLLS`]; it is killed [`KILL_POLLS`] later. The sequence is
//! plain logic: the launcher performs the steps it returns.

use ramflux_desktop_sdk::DaemonStop;

/// Polls a daemon gets to exit after a graceful stop request (10 s at the
/// launcher's 250 ms interval, the budget of `daemon.stop`).
pub const GRACEFUL_POLLS: u32 = 40;

/// Polls a terminated daemon gets before it is killed.
pub const KILL_POLLS: u32 = 12;

/// What the launcher does on this poll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopStep {
    /// Keep waiting for the process to exit.
    Wait,
    /// Send the terminate signal (the graceful stop did not work).
    Terminate,
    /// Kill the process.
    Kill,
}

/// The stop of one owned daemon.
#[derive(Debug)]
pub struct StopSequence {
    request: Option<DaemonStop>,
    polls: u32,
    terminated: bool,
    refused: Option<String>,
}

impl StopSequence {
    /// Starts with the graceful request, or with none (no live link).
    pub fn new(request: Option<DaemonStop>) -> Self {
        Self { request, polls: 0, terminated: false, refused: None }
    }

    /// One poll while the process is still running.
    pub fn step(&mut self) -> StopStep {
        self.polls += 1;
        if self.terminated {
            return if self.polls > KILL_POLLS { StopStep::Kill } else { StopStep::Wait };
        }
        let refused = match self.request.as_mut().map(DaemonStop::poll) {
            None => Some("no daemon link".to_owned()),
            Some(Some(Err(reason))) => Some(reason),
            Some(_) => None,
        };
        if refused.is_some() || self.polls >= GRACEFUL_POLLS {
            self.refused = refused;
            self.terminated = true;
            self.polls = 0;
            return StopStep::Terminate;
        }
        StopStep::Wait
    }

    /// Whether the process was only asked (no signal sent so far).
    pub fn graceful(&self) -> bool {
        !self.terminated
    }

    /// Why the graceful stop did not work, when it was refused.
    pub fn refused(&self) -> Option<&str> {
        self.refused.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_a_link_the_process_is_terminated_then_killed() {
        let mut sequence = StopSequence::new(None);
        assert_eq!(sequence.step(), StopStep::Terminate);
        assert!(!sequence.graceful());
        assert_eq!(sequence.refused(), Some("no daemon link"));
        for _ in 0..KILL_POLLS {
            assert_eq!(sequence.step(), StopStep::Wait);
        }
        assert_eq!(sequence.step(), StopStep::Kill);
    }
}
