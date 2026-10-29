// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! [`SdkSource`]: the core `Source` over the bridge, and [`SdkHub`] that
//! hands one out per scope.
//!
//! The source assigns each delivered fact the next sequence number of its
//! scope (deterministic per account: one counter, facts in bridge order) and
//! keeps the counter across reconnects, so the store's cursor keeps moving
//! forward when the bridge re-snapshots. It never blocks: `submit` and
//! `focus` queue a command, `drain` takes what has arrived.

use std::sync::mpsc as std_mpsc;

use ramflux_desktop_core::{
    AccountId, CallId, CallMedia, CeremonyPhase, ContextId, DomainEvent, Effect, EventEnvelope,
    MomentId, RecoveryPhrase, RendererProfile, Source, Timestamp,
};

use crate::bridge::{Bridge, Command, Delivery, Handoff, MediaAnswer, PhraseAudit};
use crate::bus::BusError;
use crate::config::SdkConfig;
use crate::translate::Scope;

/// Starts the bridge and creates sources for its scopes.
#[derive(Clone)]
pub struct SdkHub {
    bridge: Bridge,
    socket: String,
}

impl SdkHub {
    /// Starts the bridge thread for `config`.
    ///
    /// # Errors
    ///
    /// Returns the I/O error when the bridge thread cannot start.
    pub fn start(config: SdkConfig) -> std::io::Result<Self> {
        let socket = config.socket_path.to_string_lossy().into_owned();
        Ok(Self { bridge: Bridge::start(config)?, socket })
    }

    /// The daemon socket the hub attaches to.
    pub fn socket(&self) -> &str {
        &self.socket
    }

    /// How the recovery phrase and typed secrets were handled by the bridge
    /// (a test hook: every buffer it made must end erased).
    pub fn phrase_audit(&self) -> std::sync::Arc<PhraseAudit> {
        self.bridge.audit().clone()
    }

    /// A source for the daemon scope (status, nodes, accounts; the entry page).
    pub fn daemon_source(&self) -> SdkSource {
        SdkSource::attach(self.bridge.clone(), Scope::Daemon)
    }

    /// A source for one unlocked account.
    pub fn account_source(&self, account: AccountId) -> SdkSource {
        SdkSource::attach(self.bridge.clone(), Scope::Account(account))
    }

    /// Asks the daemon on the hub's socket to stop (`daemon.stop`: it
    /// stops accepting, drains, closes its databases and removes its
    /// socket). Returns at once; the answer arrives on the handle.
    pub fn stop_daemon(&self) -> DaemonStop {
        let (reply, answer) = std_mpsc::channel();
        self.bridge.send(Command::StopDaemon { reply });
        DaemonStop { answer, result: None }
    }
}

/// A pending `daemon.stop`.
#[derive(Debug)]
pub struct DaemonStop {
    answer: std_mpsc::Receiver<Result<(), BusError>>,
    result: Option<Result<(), String>>,
}

impl DaemonStop {
    /// The daemon's answer once it arrived: `Ok` when it accepted the stop,
    /// else why not. `None` while it is pending.
    pub fn poll(&mut self) -> Option<Result<(), String>> {
        if self.result.is_none() {
            self.result = match self.answer.try_recv() {
                Ok(answer) => Some(answer.map_err(|error| error.message())),
                Err(std_mpsc::TryRecvError::Empty) => None,
                Err(std_mpsc::TryRecvError::Disconnected) => {
                    Some(Err("the bridge stopped".to_owned()))
                }
            };
        }
        self.result.clone()
    }

    /// Waits up to `limit` for the answer (application quit only).
    pub fn wait(&mut self, limit: std::time::Duration) -> Option<Result<(), String>> {
        if self.result.is_none()
            && let Ok(answer) = self.answer.recv_timeout(limit)
        {
            self.result = Some(answer.map_err(|error| error.message()));
        }
        self.poll()
    }
}

/// The SDK-backed source of one scope.
pub struct SdkSource {
    bridge: Bridge,
    scope: Scope,
    inbox: std_mpsc::Receiver<Delivery>,
    media: (std_mpsc::Sender<MediaAnswer>, std_mpsc::Receiver<MediaAnswer>),
    last_seq: u64,
    profile: Option<RendererProfile>,
    /// Phrases the bridge moved here, not yet taken.
    phrases: std_mpsc::Receiver<Handoff>,
    /// The phrase of the running creation ceremony, once handed over.
    parked: Option<Handoff>,
    /// The creation ceremony last reported, while its phrase may be out.
    running: Option<String>,
    /// The ceremony whose phrase was given to the screen (once each).
    given: Option<String>,
    audit: std::sync::Arc<PhraseAudit>,
}

impl std::fmt::Debug for SdkSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SdkSource")
            .field("scope", &self.scope)
            .field("last_seq", &self.last_seq)
            .finish()
    }
}

impl SdkSource {
    fn attach(bridge: Bridge, scope: Scope) -> Self {
        let (outbox, inbox) = std_mpsc::channel();
        let (handoff, phrases) = std_mpsc::channel();
        bridge.send(Command::Attach { scope: scope.clone(), outbox, phrases: handoff });
        let audit = bridge.audit().clone();
        Self {
            bridge,
            scope,
            inbox,
            media: std_mpsc::channel(),
            last_seq: 0,
            profile: None,
            phrases,
            parked: None,
            running: None,
            given: None,
            audit,
        }
    }

    /// Moves what the bridge handed over into the one parked slot, erasing
    /// every phrase that is not the running ceremony's (an earlier
    /// ceremony's, a second of the same, or one that ended unseen).
    fn settle_phrases(&mut self) {
        while let Ok(handoff) = self.phrases.try_recv() {
            let wanted = self.running.as_deref() == Some(handoff.ceremony_id.as_str())
                && self.given.as_deref() != Some(handoff.ceremony_id.as_str());
            if wanted {
                if let Some(older) = self.parked.replace(handoff) {
                    self.audit.wipe(older.phrase);
                }
            } else {
                self.audit.wipe(handoff.phrase);
            }
        }
        let stale = self
            .parked
            .as_ref()
            .is_some_and(|parked| self.running.as_deref() != Some(parked.ceremony_id.as_str()));
        if stale && let Some(parked) = self.parked.take() {
            self.audit.wipe(parked.phrase);
        }
    }

    /// The scope served.
    pub fn scope(&self) -> &Scope {
        &self.scope
    }

    /// The last sequence number handed out (the scope's cursor).
    pub fn last_seq(&self) -> u64 {
        self.last_seq
    }

    /// The renderer profile the app declared.
    pub fn negotiated_profile(&self) -> Option<&RendererProfile> {
        self.profile.as_ref()
    }
}

impl Source for SdkSource {
    fn submit(&mut self, effect: Effect) {
        self.bridge.send(Command::Effect { scope: self.scope.clone(), effect });
    }

    fn drain(&mut self, _now: Timestamp) -> Vec<EventEnvelope> {
        let mut envelopes = Vec::new();
        while let Ok(delivery) = self.inbox.try_recv() {
            self.last_seq += 1;
            if let DomainEvent::CeremonyUpdated(ceremony) = &delivery.event {
                // The words are the screen's only while the phrase is out:
                // from the hide on (or once the ceremony ends) they are not.
                let showing =
                    matches!(ceremony.phase, CeremonyPhase::Prepared | CeremonyPhase::PhraseReady);
                self.running = showing.then(|| ceremony.ceremony_id.clone());
                if matches!(ceremony.phase, CeremonyPhase::Prepared) {
                    self.given = None;
                }
            }
            envelopes.push(EventEnvelope::new(self.last_seq, delivery.at, delivery.event));
        }
        // After the events, so a phrase that arrived with its ceremony is
        // kept and one whose ceremony ended meanwhile is erased now.
        self.settle_phrases();
        envelopes
    }

    fn focus(&mut self, context: Option<&ContextId>) {
        self.bridge.send(Command::Focus { scope: self.scope.clone(), context: context.cloned() });
    }

    /// Kept for the surface stream (K2), which will send it to producers;
    /// the daemon has no counterpart yet.
    fn renderer_profile(&mut self, profile: &RendererProfile) {
        self.profile = Some(profile.clone());
    }

    fn request_bodies(&mut self, moments: &[MomentId]) {
        if !moments.is_empty() {
            self.bridge
                .send(Command::Bodies { scope: self.scope.clone(), moments: moments.to_vec() });
        }
    }

    fn request_call_media(&mut self, call: &CallId) {
        self.bridge.send(Command::CallMedia {
            scope: self.scope.clone(),
            call_id: call.clone(),
            reply: self.media.0.clone(),
        });
    }

    fn take_call_media(&mut self) -> Option<(CallId, Result<CallMedia, String>)> {
        self.media.1.try_recv().ok()
    }

    /// The phrase the bridge moved here for the running creation ceremony,
    /// once. It is parsed in place (no copy); a phrase that is not 24 list
    /// words is erased and never shown.
    fn take_recovery_phrase(&mut self) -> Option<RecoveryPhrase> {
        self.settle_phrases();
        let handoff = self.parked.take()?;
        self.given = Some(handoff.ceremony_id);
        match RecoveryPhrase::parse(handoff.phrase) {
            Ok(phrase) => {
                self.audit.take();
                Some(phrase)
            }
            Err(_) => {
                // Dropped inside `parse`, which erases it.
                self.audit.reject();
                None
            }
        }
    }
}

impl Drop for SdkSource {
    fn drop(&mut self) {
        self.running = None;
        self.settle_phrases();
        self.bridge.send(Command::Detach { scope: self.scope.clone() });
    }
}
