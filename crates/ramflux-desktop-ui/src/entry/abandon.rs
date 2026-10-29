// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! A begin the user backed out of while it was in flight.
//!
//! The daemon may still answer it with a ceremony, so the panel must let go
//! of that ceremony as soon as it shows, and may not start another meanwhile.
//! The wait is tracked apart from the panel's `pending` request: it outlives
//! a reopen of the panel (which clears `pending`) and the failure of its own
//! requests. It ends however the begin's answer arrives:
//!
//! | The begin was answered with | The wait ends |
//! |---|---|
//! | a ceremony | after the cancel that lets go of it is sent and answered |
//! | a refusal that is a ceremony update (it ended nothing) | at once |
//! | a failure of the begin itself, no ceremony update | at once |
//!
//! and the cancel's own failure also ends it (the ceremony, if it still
//! runs, is then the panel's own to see and cancel again).

/// The wait for an abandoned begin, with the failure counts at which the
/// begin and the cancel were sent (a later failure of either is its answer).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Abandon {
    /// The failures total when the abandoned begin was sent.
    pub(crate) begin_mark: u64,
    /// The failures total when the cancel was sent, once it was.
    pub(crate) cancel_mark: Option<u64>,
}

/// What the store shows of the abandoned begin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Arrival {
    /// A ceremony runs in the store.
    pub(crate) running: bool,
    /// The ceremony record changed since the panel last looked.
    pub(crate) answered: bool,
    /// The panel has no request in flight (it was reopened, or the answer
    /// cleared it).
    pub(crate) idle: bool,
    /// The begin failed (a failure of its method after the begin was sent).
    pub(crate) begin_failed: bool,
    /// The cancel failed (a failure of its method after it was sent).
    pub(crate) cancel_failed: bool,
}

/// What the panel does next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Next {
    /// Keep waiting.
    Wait,
    /// Send the cancel that lets go of the ceremony.
    Cancel,
    /// The wait is over; records older than now are not this panel's.
    Release,
    /// The wait is over, but the ceremony that still runs is the panel's
    /// own to show (the cancel failed).
    ReleaseShowing,
}

impl Abandon {
    /// Decides from what arrived.
    pub(crate) fn next(&self, arrival: Arrival) -> Next {
        if arrival.cancel_failed {
            return Next::ReleaseShowing;
        }
        if arrival.running && self.cancel_mark.is_none() && (arrival.answered || arrival.idle) {
            return Next::Cancel;
        }
        if !arrival.running && (arrival.answered || arrival.begin_failed) {
            return Next::Release;
        }
        Next::Wait
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WAITING: Abandon = Abandon { begin_mark: 3, cancel_mark: None };
    const CANCELLING: Abandon = Abandon { begin_mark: 3, cancel_mark: Some(5) };

    fn arrival() -> Arrival {
        Arrival {
            running: false,
            answered: false,
            idle: false,
            begin_failed: false,
            cancel_failed: false,
        }
    }

    #[test]
    fn nothing_arrived_nothing_happens_whether_or_not_the_panel_was_reopened() {
        assert_eq!(WAITING.next(arrival()), Next::Wait);
        assert_eq!(WAITING.next(Arrival { idle: true, ..arrival() }), Next::Wait);
    }

    #[test]
    fn a_ceremony_that_shows_is_cancelled_once() {
        // The begin's own answer, with the panel still waiting.
        let shown = Arrival { running: true, answered: true, ..arrival() };
        assert_eq!(WAITING.next(shown), Next::Cancel);
        // The same, after the panel was reopened (nothing in flight).
        assert_eq!(WAITING.next(Arrival { running: true, idle: true, ..arrival() }), Next::Cancel);
        // Once the cancel is sent, it is not sent again.
        assert_eq!(CANCELLING.next(shown), Next::Wait);
        assert_eq!(CANCELLING.next(Arrival { running: true, idle: true, ..arrival() }), Next::Wait);
    }

    #[test]
    fn the_cancels_answer_ends_the_wait() {
        let cancelled = Arrival { answered: true, ..arrival() };
        assert_eq!(CANCELLING.next(cancelled), Next::Release);
    }

    #[test]
    fn a_refusal_that_is_a_ceremony_update_ends_the_wait_at_once() {
        assert_eq!(WAITING.next(Arrival { answered: true, ..arrival() }), Next::Release);
        let reopened = Arrival { answered: true, idle: true, ..arrival() };
        assert_eq!(WAITING.next(reopened), Next::Release);
    }

    #[test]
    fn a_failure_of_the_begin_itself_ends_the_wait_even_after_a_reopen() {
        // No ceremony update at all: only the failure says the begin is over.
        let failed = Arrival { begin_failed: true, ..arrival() };
        assert_eq!(WAITING.next(failed), Next::Release);
        assert_eq!(WAITING.next(Arrival { idle: true, ..failed }), Next::Release);
    }

    #[test]
    fn a_failure_of_the_cancel_ends_the_wait_and_shows_what_still_runs() {
        let failed = Arrival { running: true, cancel_failed: true, ..arrival() };
        assert_eq!(CANCELLING.next(failed), Next::ReleaseShowing);
        assert_eq!(CANCELLING.next(Arrival { idle: true, ..failed }), Next::ReleaseShowing);
        // Whatever else came with it.
        let both = Arrival { begin_failed: true, answered: true, ..failed };
        assert_eq!(CANCELLING.next(both), Next::ReleaseShowing);
    }

    #[test]
    fn a_running_ceremony_never_ends_the_wait_by_a_begin_failure_alone() {
        // A begin failure while a ceremony runs is another request's: cancel it.
        let odd = Arrival { running: true, begin_failed: true, ..arrival() };
        assert_eq!(WAITING.next(odd), Next::Wait);
        assert_eq!(WAITING.next(Arrival { idle: true, ..odd }), Next::Cancel);
    }
}
