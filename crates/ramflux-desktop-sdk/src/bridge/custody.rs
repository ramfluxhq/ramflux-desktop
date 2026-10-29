// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Creation and recovery ceremonies (born-v3), on a connection of their own.
//!
//! A ceremony belongs to one connection: the daemon erases it when the
//! connection ends. So this module keeps one actor per scope that owns the
//! only [`LocalBusCreationClient`] (or [`LocalBusRecoveryClient`]) of its
//! ceremony, never shared with the multiplexed request connection, and runs
//! its steps in order. Dropping the actor (the scope ends, the window closes)
//! drops the client and so cancels the ceremony.
//!
//! What is secret never becomes a JSON value, an event, a log line or a
//! `String` the bridge keeps:
//!
//! - **The recovery phrase** comes out of the client as a `Zeroizing<String>`
//!   and is moved, untouched, through a plain channel to the GPUI side
//!   ([`Handoff`], taken by [`crate::SdkSource`]). The ceremony event that
//!   says the phrase is ready is sent after the move.
//! - **The words, the passphrase and a typed recovery phrase** arrive in
//!   [`ramflux_desktop_core::SecretInput`]s, are copied once into zeroizing
//!   buffers for the request, and both are erased when the request ends.
//!
//! Every such buffer is counted by [`PhraseAudit`], the test hook that lets
//! a test assert that each one was erased.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc as std_mpsc;
use std::time::Duration;

use ramflux_desktop_core::{
    AccountId, CeremonyAccount, CeremonyFailure, CeremonyPhase, DomainEvent, EffectFailure,
    FailureCode, Networking, RecoveryCeremony, RecoveryPhase, SecretInput, Timestamp, methods,
};
use ramflux_sdk::custody::CreationState;
use ramflux_sdk::{LocalBusCreationClient, LocalBusRecoveryClient, SdkError};
use tokio::sync::mpsc;
use zeroize::{Zeroize, Zeroizing};

use super::now;
use super::scope::Internal;
use crate::plan::fetch::Fetch;

/// How the recovery phrase and the typed secrets were handled: a test hook.
///
/// The counters only grow. `issued` phrases are ones the daemon handed out;
/// each ends as `taken` (moved to the screen, which owns erasing it) or
/// `wiped` (erased here, and checked empty). `secrets` buffers are the
/// zeroizing copies of typed words and passphrases made for a request; each
/// ends `secrets_wiped`. A test reads them to assert that nothing the bridge
/// made is left unerased.
#[derive(Debug, Default)]
pub struct PhraseAudit {
    issued: AtomicUsize,
    taken: AtomicUsize,
    wiped: AtomicUsize,
    secrets: AtomicUsize,
    secrets_wiped: AtomicUsize,
}

impl PhraseAudit {
    /// Phrases the daemon handed out.
    pub fn issued(&self) -> usize {
        self.issued.load(Ordering::SeqCst)
    }

    /// Phrases moved to the screen.
    pub fn taken(&self) -> usize {
        self.taken.load(Ordering::SeqCst)
    }

    /// Phrases erased by the bridge without reaching the screen.
    pub fn wiped(&self) -> usize {
        self.wiped.load(Ordering::SeqCst)
    }

    /// Phrases neither taken nor erased yet (on their way, or parked).
    pub fn outstanding(&self) -> usize {
        self.issued().saturating_sub(self.taken() + self.wiped())
    }

    /// Zeroizing buffers made for typed secrets.
    pub fn secrets(&self) -> usize {
        self.secrets.load(Ordering::SeqCst)
    }

    /// Of those, the ones erased (and checked empty) so far.
    pub fn secrets_wiped(&self) -> usize {
        self.secrets_wiped.load(Ordering::SeqCst)
    }

    pub(crate) fn issue(&self) {
        self.issued.fetch_add(1, Ordering::SeqCst);
    }

    /// A phrase that did not parse was erased by the parser.
    pub(crate) fn reject(&self) {
        self.wiped.fetch_add(1, Ordering::SeqCst);
    }

    pub(crate) fn take(&self) {
        self.taken.fetch_add(1, Ordering::SeqCst);
    }

    /// Erases `phrase` and counts it, if it is empty afterwards.
    pub(crate) fn wipe(&self, mut phrase: Zeroizing<String>) {
        phrase.zeroize();
        if phrase.is_empty() {
            self.wiped.fetch_add(1, Ordering::SeqCst);
        }
    }
}

/// A phrase on its way from the bridge to the screen.
pub(crate) struct Handoff {
    /// The ceremony it belongs to.
    pub(crate) ceremony_id: String,
    /// The words, moved, never copied.
    pub(crate) phrase: Zeroizing<String>,
}

/// Typed secrets copied for one request: erased and counted on drop.
struct Secrets<const N: usize> {
    buffers: [Zeroizing<String>; N],
    audit: Arc<PhraseAudit>,
}

impl<const N: usize> Secrets<N> {
    fn new(typed: [&SecretInput; N], audit: &Arc<PhraseAudit>) -> Self {
        audit.secrets.fetch_add(N, Ordering::SeqCst);
        Self {
            buffers: typed.map(|secret| Zeroizing::new(secret.expose().to_owned())),
            audit: audit.clone(),
        }
    }
}

impl<const N: usize> Drop for Secrets<N> {
    fn drop(&mut self) {
        for buffer in &mut self.buffers {
            buffer.zeroize();
            if buffer.is_empty() {
                self.audit.secrets_wiped.fetch_add(1, Ordering::SeqCst);
            }
        }
    }
}

/// One step of a ceremony, as the scope's task sends it.
pub(super) enum Step {
    /// `account.create`: open the connection and begin.
    Begin { label: String },
    /// `account.create.show_once`.
    Show,
    /// `account.create.hidden_challenge`.
    Hide,
    /// `account.create.confirm`: the three words, then the passphrase.
    Confirm { secrets: Vec<SecretInput> },
    /// `account.create.cancel`.
    Cancel,
    /// `account.recover.begin`: open the connection and begin.
    RecoverBegin { account_id: String },
    /// `account.recover.confirm`: the phrase, then the passphrase.
    RecoverConfirm { anchor: String, secrets: Vec<SecretInput> },
    /// `account.recover.cancel`.
    RecoverCancel,
}

impl Step {
    /// The step an effect asks for, if it is a ceremony method.
    pub(super) fn of(
        effect: ramflux_desktop_core::Effect,
    ) -> Result<Self, ramflux_desktop_core::Effect> {
        use ramflux_desktop_core::Effect;
        let text = |body: &serde_json::Value, key: &str| {
            body.get(key).and_then(serde_json::Value::as_str).unwrap_or_default().to_owned()
        };
        match effect {
            // Begin is `account.create` without a secret; the one-shot
            // create (which carries the passphrase) is another call.
            Effect::Call { method, body } if method == methods::CUSTODY_BEGIN => {
                Ok(Self::Begin { label: text(&body, "display_name") })
            }
            Effect::Call { method, .. } if method == methods::CUSTODY_SHOW_ONCE => Ok(Self::Show),
            Effect::Call { method, .. } if method == methods::CUSTODY_HIDDEN_CHALLENGE => {
                Ok(Self::Hide)
            }
            Effect::Call { method, .. } if method == methods::CUSTODY_CANCEL => Ok(Self::Cancel),
            Effect::SecretCall { method, secrets, .. } if method == methods::CUSTODY_CONFIRM => {
                Ok(Self::Confirm { secrets })
            }
            Effect::Call { method, body } if method == methods::RECOVER_BEGIN => {
                Ok(Self::RecoverBegin { account_id: text(&body, "account_id") })
            }
            Effect::SecretCall { method, body, secrets } if method == methods::RECOVER_CONFIRM => {
                Ok(Self::RecoverConfirm { anchor: text(&body, "confirmed_anchor"), secrets })
            }
            Effect::Call { method, .. } if method == methods::RECOVER_CANCEL => {
                Ok(Self::RecoverCancel)
            }
            other => Err(other),
        }
    }

    /// The contract method this step performs (its failures are named by it).
    fn method(&self) -> &'static str {
        match self {
            Self::Begin { .. } => methods::CUSTODY_BEGIN,
            Self::Show => methods::CUSTODY_SHOW_ONCE,
            Self::Hide => methods::CUSTODY_HIDDEN_CHALLENGE,
            Self::Confirm { .. } => methods::CUSTODY_CONFIRM,
            Self::Cancel => methods::CUSTODY_CANCEL,
            Self::RecoverBegin { .. } => methods::RECOVER_BEGIN,
            Self::RecoverConfirm { .. } => methods::RECOVER_CONFIRM,
            Self::RecoverCancel => methods::RECOVER_CANCEL,
        }
    }
}

/// An account a ceremony made or restored, as the daemon reports it.
pub(crate) struct NativeAccount {
    pub(crate) id: AccountId,
    pub(crate) display_name: String,
    pub(crate) networking: Networking,
}

/// What a step came to.
pub(super) struct Outcome {
    /// The ceremony events (the account comes before them).
    pub(super) events: Vec<DomainEvent>,
    /// The account that exists now, for the translator to record.
    pub(super) account: Option<NativeAccount>,
    /// Reads to follow.
    pub(super) refresh: Vec<Fetch>,
}

impl Outcome {
    fn events(events: Vec<DomainEvent>) -> Self {
        Self { events, account: None, refresh: Vec::new() }
    }
}

/// The scope's handle on its ceremony actor.
pub(super) struct Custody {
    steps: Option<mpsc::UnboundedSender<Step>>,
}

impl Custody {
    pub(super) fn new() -> Self {
        Self { steps: None }
    }

    /// Queues a step, starting the actor on first use.
    pub(super) fn send(&mut self, step: Step, context: &Context) {
        let steps = self.steps.get_or_insert_with(|| spawn(context.clone()));
        let _ = steps.send(step);
    }
}

/// What the actor needs from its scope.
#[derive(Clone)]
pub(super) struct Context {
    pub(super) socket: std::path::PathBuf,
    pub(super) budget: Duration,
    pub(super) internal: mpsc::UnboundedSender<Internal>,
    pub(super) phrases: std_mpsc::Sender<Handoff>,
    pub(super) audit: Arc<PhraseAudit>,
}

enum Session {
    Idle,
    Creation { client: LocalBusCreationClient, meta: Meta },
    Recovery { client: LocalBusRecoveryClient, meta: RecoveryMeta },
}

/// The public facts of a running creation.
#[derive(Clone)]
struct Meta {
    ceremony_id: String,
    label: String,
    expires_at: Timestamp,
}

/// The public facts of a running recovery.
#[derive(Clone)]
struct RecoveryMeta {
    account_id: AccountId,
    anchor: String,
    ceremony_id: String,
    expires_at: Timestamp,
}

fn spawn(context: Context) -> mpsc::UnboundedSender<Step> {
    let (steps, mut inbox) = mpsc::unbounded_channel::<Step>();
    tokio::spawn(async move {
        let mut session = Session::Idle;
        while let Some(step) = inbox.recv().await {
            let method = step.method();
            let outcome =
                match tokio::time::timeout(context.budget, run(step, &mut session, &context)).await
                {
                    Ok(outcome) => outcome,
                    Err(_) => {
                        // The answer did not come: the connection (and the
                        // ceremony with it) is let go of.
                        let ended = end(&mut session, CeremonyFailure::Invalid);
                        Outcome::events(
                            ended
                                .into_iter()
                                .chain([failed(
                                    method,
                                    FailureCode::Timeout,
                                    crate::bus::TIMEOUT_KEY,
                                )])
                                .collect(),
                        )
                    }
                };
            let _ = context.internal.send(Internal::Custody(outcome));
        }
        // The scope ended: the client is dropped here, and the daemon
        // erases the ceremony with the connection.
    });
    steps
}

fn failed(method: &str, code: FailureCode, message: &str) -> DomainEvent {
    DomainEvent::EffectFailed(EffectFailure {
        method: method.to_owned(),
        code,
        message: message.to_owned(),
    })
}

/// The failure the user is told about, for a custody refusal; `None` for a
/// connection that broke (no refusal came).
fn refusal(error: &SdkError) -> Option<CeremonyFailure> {
    Some(match error {
        SdkError::CustodyConfirmationFailed => CeremonyFailure::ConfirmationFailed,
        SdkError::CustodyVaultUnavailable => CeremonyFailure::VaultUnavailable,
        SdkError::CustodyExpired => CeremonyFailure::Expired,
        SdkError::CustodySetupInvalid => CeremonyFailure::Invalid,
        _ => return None,
    })
}

fn networking(state: CreationState) -> Networking {
    match state {
        CreationState::Live => Networking::Live,
        CreationState::NetworkingPending => Networking::Pending,
    }
}

fn creation(meta: &Meta, phase: CeremonyPhase) -> DomainEvent {
    DomainEvent::CeremonyUpdated(ramflux_desktop_core::Ceremony {
        phase,
        ceremony_id: meta.ceremony_id.clone(),
        label: meta.label.clone(),
        expires_at: meta.expires_at,
    })
}

fn recovery(meta: &RecoveryMeta, phase: RecoveryPhase) -> DomainEvent {
    DomainEvent::RecoveryUpdated(RecoveryCeremony {
        phase,
        account_id: meta.account_id.clone(),
        anchor: meta.anchor.clone(),
        ceremony_id: meta.ceremony_id.clone(),
        expires_at: meta.expires_at,
    })
}

/// Ends whatever ceremony the session holds, dropping its connection, with
/// the event that says why.
fn end(session: &mut Session, failure: CeremonyFailure) -> Vec<DomainEvent> {
    match std::mem::replace(session, Session::Idle) {
        Session::Idle => Vec::new(),
        Session::Creation { meta, .. } => {
            vec![creation(&meta, CeremonyPhase::Failed(failure))]
        }
        Session::Recovery { meta, .. } => vec![recovery(&meta, RecoveryPhase::Failed(failure))],
    }
}

/// A refused begin: a failure with no ceremony behind it.
fn refused_creation(label: &str, failure: CeremonyFailure) -> DomainEvent {
    creation(
        &Meta { ceremony_id: String::new(), label: label.to_owned(), expires_at: Timestamp(0) },
        CeremonyPhase::Failed(failure),
    )
}

fn refused_recovery(account_id: &str, failure: CeremonyFailure) -> DomainEvent {
    recovery(
        &RecoveryMeta {
            account_id: AccountId::new(account_id),
            anchor: String::new(),
            ceremony_id: String::new(),
            expires_at: Timestamp(0),
        },
        RecoveryPhase::Failed(failure),
    )
}

/// A step that failed after the ceremony began: its connection is dropped
/// (the daemon erases the ceremony) and the failure is told.
fn broke(method: &str, error: &SdkError, session: &mut Session) -> Outcome {
    let mut events;
    if let Some(failure) = refusal(error) {
        events = end(session, failure);
    } else {
        // The connection broke: no answer will come, and the ceremony cannot
        // be resumed from another connection.
        events = end(session, CeremonyFailure::Invalid);
        events.push(failed(method, FailureCode::Transport, &error.to_string()));
    }
    Outcome::events(events)
}

async fn run(step: Step, session: &mut Session, context: &Context) -> Outcome {
    let method = step.method();
    let outcome = match step {
        Step::Begin { label } => begin(&label, session, context).await,
        Step::Show => show(session, context).await,
        Step::Hide => hide(session).await,
        Step::Confirm { secrets } => confirm(&secrets, session, context).await,
        Step::Cancel => cancel_creation(session).await,
        Step::RecoverBegin { account_id } => begin_recovery(&account_id, session, context).await,
        Step::RecoverConfirm { anchor, secrets } => {
            confirm_recovery(&anchor, &secrets, session, context).await
        }
        Step::RecoverCancel => cancel_recovery(session).await,
    };
    // A step with no ceremony to act on is refused.
    outcome.unwrap_or_else(|| {
        Outcome::events(vec![failed(
            method,
            FailureCode::Daemon("CustodySetupInvalid".to_owned()),
            crate::translate::reason::key::CEREMONY_REFUSED,
        )])
    })
}

async fn begin(label: &str, session: &mut Session, context: &Context) -> Option<Outcome> {
    // One ceremony at a time: a running one is let go of (its connection
    // closes) before the next begins.
    *session = Session::Idle;
    let mut client = match LocalBusCreationClient::connect(&context.socket).await {
        Ok(client) => client,
        Err(error) => {
            return Some(Outcome::events(vec![failed(
                methods::CUSTODY_BEGIN,
                FailureCode::Transport,
                &error.to_string(),
            )]));
        }
    };
    match client.begin(label, ramflux_protocol::PrincipalKind::Human).await {
        Ok(pending) => {
            let meta = Meta {
                ceremony_id: pending.ceremony_id,
                label: label.to_owned(),
                expires_at: now().saturating_add_ms(pending.expires_in_ms),
            };
            let event = creation(&meta, CeremonyPhase::Prepared);
            *session = Session::Creation { client, meta };
            Some(Outcome::events(vec![event]))
        }
        Err(error) => Some(Outcome::events(match refusal(&error) {
            Some(failure) => vec![refused_creation(label, failure)],
            None => {
                vec![failed(methods::CUSTODY_BEGIN, FailureCode::Transport, &error.to_string())]
            }
        })),
    }
}

async fn show(session: &mut Session, context: &Context) -> Option<Outcome> {
    let Session::Creation { client, meta } = session else { return None };
    match client.show_once().await {
        Ok(phrase) => {
            // Moved to the screen's side before the event says it is ready.
            context.audit.issue();
            let handoff = Handoff { ceremony_id: meta.ceremony_id.clone(), phrase };
            if let Err(std_mpsc::SendError(lost)) = context.phrases.send(handoff) {
                context.audit.wipe(lost.phrase);
            }
            Some(Outcome::events(vec![creation(meta, CeremonyPhase::PhraseReady)]))
        }
        Err(error) => Some(broke(methods::CUSTODY_SHOW_ONCE, &error, session)),
    }
}

async fn hide(session: &mut Session) -> Option<Outcome> {
    let Session::Creation { client, meta } = session else { return None };
    match client.hidden_challenge().await {
        Ok(positions) => {
            let event = creation(meta, CeremonyPhase::Challenged { positions });
            Some(Outcome::events(vec![event]))
        }
        Err(error) => Some(broke(methods::CUSTODY_HIDDEN_CHALLENGE, &error, session)),
    }
}

async fn confirm(
    secrets: &[SecretInput],
    session: &mut Session,
    context: &Context,
) -> Option<Outcome> {
    let Session::Creation { client, meta } = session else { return None };
    let [first, second, third, passphrase] = secrets else {
        return Some(broke(methods::CUSTODY_CONFIRM, &SdkError::CustodySetupInvalid, session));
    };
    let words = Secrets::new([first, second, third], &context.audit);
    let passphrase = Secrets::new([passphrase], &context.audit);
    let result = client.confirm(&words.buffers, &passphrase.buffers[0]).await;
    drop((words, passphrase));
    match result {
        Ok(created) => {
            let account = NativeAccount {
                id: AccountId::new(created.account_id),
                display_name: created.display_name,
                networking: networking(created.networking),
            };
            let done = CeremonyAccount {
                local_account_id: account.id.to_string(),
                display_name: account.display_name.clone(),
                networking: account.networking,
            };
            let event = creation(meta, CeremonyPhase::Created(done));
            // The ceremony is consumed: its connection is let go of.
            *session = Session::Idle;
            Some(Outcome {
                events: vec![event],
                account: Some(account),
                refresh: vec![Fetch::AccountList],
            })
        }
        Err(error) => Some(broke(methods::CUSTODY_CONFIRM, &error, session)),
    }
}

async fn cancel_creation(session: &mut Session) -> Option<Outcome> {
    let Session::Creation { client, meta } = session else { return None };
    // Best effort: the connection is dropped either way, and the daemon
    // erases the ceremony with it.
    let _ = client.cancel().await;
    let event = creation(meta, CeremonyPhase::Cancelled);
    *session = Session::Idle;
    Some(Outcome::events(vec![event]))
}

async fn begin_recovery(
    account_id: &str,
    session: &mut Session,
    context: &Context,
) -> Option<Outcome> {
    *session = Session::Idle;
    let mut client = match LocalBusRecoveryClient::connect(&context.socket).await {
        Ok(client) => client,
        Err(error) => {
            return Some(Outcome::events(vec![failed(
                methods::RECOVER_BEGIN,
                FailureCode::Transport,
                &error.to_string(),
            )]));
        }
    };
    match client.begin(account_id).await {
        Ok(pending) => {
            let meta = RecoveryMeta {
                account_id: AccountId::new(account_id),
                anchor: pending.account_anchor,
                ceremony_id: pending.ceremony_id,
                expires_at: now().saturating_add_ms(pending.expires_in_ms),
            };
            let event = recovery(&meta, RecoveryPhase::Prepared);
            *session = Session::Recovery { client, meta };
            Some(Outcome::events(vec![event]))
        }
        Err(error) => Some(Outcome::events(match refusal(&error) {
            Some(failure) => vec![refused_recovery(account_id, failure)],
            None => {
                vec![failed(methods::RECOVER_BEGIN, FailureCode::Transport, &error.to_string())]
            }
        })),
    }
}

async fn confirm_recovery(
    anchor: &str,
    secrets: &[SecretInput],
    session: &mut Session,
    context: &Context,
) -> Option<Outcome> {
    let Session::Recovery { client, meta } = session else { return None };
    let [phrase, passphrase] = secrets else {
        return Some(broke(methods::RECOVER_CONFIRM, &SdkError::CustodySetupInvalid, session));
    };
    let phrase = Secrets::new([phrase], &context.audit);
    let passphrase = Secrets::new([passphrase], &context.audit);
    let result = client.confirm(&phrase.buffers[0], &passphrase.buffers[0], anchor).await;
    drop((phrase, passphrase));
    match result {
        Ok(opened) => {
            let account = NativeAccount {
                id: AccountId::new(opened.account_id),
                display_name: opened.display_name,
                networking: networking(opened.networking),
            };
            let done = CeremonyAccount {
                local_account_id: account.id.to_string(),
                display_name: account.display_name.clone(),
                networking: account.networking,
            };
            let event = recovery(meta, RecoveryPhase::Recovered(done));
            *session = Session::Idle;
            Some(Outcome {
                events: vec![event],
                account: Some(account),
                refresh: vec![Fetch::AccountList],
            })
        }
        Err(error) => Some(broke(methods::RECOVER_CONFIRM, &error, session)),
    }
}

async fn cancel_recovery(session: &mut Session) -> Option<Outcome> {
    let Session::Recovery { client, meta } = session else { return None };
    let _ = client.cancel().await;
    let event = recovery(meta, RecoveryPhase::Cancelled);
    *session = Session::Idle;
    Some(Outcome::events(vec![event]))
}
