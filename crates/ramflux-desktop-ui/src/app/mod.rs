// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The application layer: [`StoreHandle`], the single writer.
//!
//! `StoreHandle` owns one [`AccountSession`] (store plus source) per account
//! and a GPUI timer task that advances every session in real time. After a
//! batch of envelopes is applied it emits [`StoreEvent::Changed`] with the
//! merged [`ChangeSet`]; each view subscribes and notifies itself only when
//! its own area changed. Views read the active store through
//! [`StoreHandle::store`] and never mutate it.
//!
//! In live mode ([`StoreHandle::attach_live`]) a daemon session (the entry
//! page's store: daemon status, nodes, accounts) runs beside one session per
//! unlocked account; sessions appear when the daemon reports an account
//! unlocked and disappear when it reports it locked. See `live`.

mod call_audio;
mod changes;
mod inspector;
mod live;
mod names;
mod renderer_errors;
mod session;

use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use gpui_kit::{Context, EventEmitter, Task};
use ramflux_desktop_core::{
    AccountId, AttentionItem, CallState, ContextId, DomainEvent, Intent, MomentId, Store,
    Timestamp, selectors,
};
use ramflux_desktop_test::{Scenario, load_fixture};

pub use call_audio::{
    CallAudio, CallAudioView, CallQuality, DeviceAudio, LiveCall, LiveStatus, NO_AUDIO,
    media_session,
};
pub use changes::ChangeSet;
pub use inspector::{INSPECTOR_CAP, Inspector, InspectorEntry, UiOutcome};
pub use names::{
    effect_label, event_name, format_path, format_scope, intent_kind_name, intent_name,
    ui_event_name,
};
pub use renderer_errors::{MAX_RENDERER_ERRORS, RendererErrors};
pub use session::{AccountSession, Applied, DispatchOutcome, audit_label};

/// Interval of the clock task.
pub const TICK: Duration = Duration::from_millis(50);

/// Account slot used for scenarios that declare no account.
pub const LOCAL_ACCOUNT: &str = "local";

static EMPTY_STORE: LazyLock<Store> = LazyLock::new(Store::new);

/// Emitted after the store handle changed state.
#[derive(Debug, Clone)]
pub enum StoreEvent {
    /// The active account's store (or clock) changed.
    Changed(Rc<ChangeSet>),
    /// A background account's store changed (attention counts only).
    BackgroundChanged(AccountId),
    /// The inspector log grew without a store change.
    Inspected,
    /// The current call's audio changed (started, failed, muted).
    CallAudio,
}

/// One "needs me" item in any account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountAttention {
    /// Account whose store holds the moment.
    pub account: AccountId,
    /// The attention item.
    pub item: AttentionItem,
}

/// Owns every account session and is the only path to the store and source.
pub struct StoreHandle {
    sessions: BTreeMap<AccountId, AccountSession>,
    active: Option<AccountId>,
    inspector: Inspector,
    clock: Option<Task<()>>,
    last_tick: Option<Instant>,
    wall_clock: bool,
    live: Option<live::LiveLink>,
    /// The daemon session's connection as last announced (a status change
    /// of the daemon is announced even while an account session is shown).
    daemon_status: Option<ramflux_desktop_core::SourceStatus>,
    call_audio: Option<Rc<dyn CallAudio>>,
    call: Option<call_audio::CallLink>,
    ringer: Option<Box<dyn std::any::Any>>,
    ringing: bool,
}

impl EventEmitter<StoreEvent> for StoreHandle {}

impl Default for StoreHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl StoreHandle {
    /// Creates a handle with no account and a stopped clock.
    pub fn new() -> Self {
        Self {
            sessions: BTreeMap::new(),
            active: None,
            inspector: Inspector::default(),
            clock: None,
            last_tick: None,
            wall_clock: false,
            live: None,
            daemon_status: None,
            call_audio: None,
            call: None,
            ringer: None,
            ringing: false,
        }
    }

    /// Anchors scenarios loaded from now on to the wall clock: offset 0 is
    /// the moment of loading, so moment times are real local times. Tests and
    /// reference screenshots keep the fixture epoch for determinism.
    pub fn set_wall_clock_anchor(&mut self, enabled: bool) {
        self.wall_clock = enabled;
    }

    fn new_session(&self, scenario: &Scenario) -> AccountSession {
        if self.wall_clock {
            AccountSession::with_epoch(scenario, wall_clock_now())
        } else {
            AccountSession::new(scenario)
        }
    }

    /// Plays `scenario` as the active account and its companions as
    /// background accounts. Every other session is dropped: a scenario is a
    /// whole world.
    pub fn load_scenario(&mut self, scenario: &Scenario, cx: &mut Context<Self>) {
        let account = scenario_account(scenario);
        self.live = None;
        self.sessions.clear();
        self.sessions.insert(account.clone(), self.new_session(scenario));
        for companion in &scenario.companions {
            if let Ok(other) = load_fixture(&companion.scenario) {
                let session = self.new_session(&other);
                self.sessions.insert(companion.account_id.clone(), session);
            }
        }
        self.active = Some(account);
        self.inspector.clear();
        self.emit_reset(cx);
    }

    /// Adds `scenario` as a background account under `account`.
    pub fn add_background_account(
        &mut self,
        account: AccountId,
        scenario: &Scenario,
        cx: &mut Context<Self>,
    ) {
        let session = self.new_session(scenario);
        self.sessions.insert(account.clone(), session);
        if self.active.is_none() {
            self.active = Some(account);
            self.emit_reset(cx);
        }
    }

    /// Switches the rendered account. Switching keeps every store; nothing reloads.
    pub fn set_active_account(&mut self, account: &AccountId, cx: &mut Context<Self>) {
        if self.sessions.contains_key(account) && self.active.as_ref() != Some(account) {
            self.active = Some(account.clone());
            self.emit_reset(cx);
        }
    }

    /// Every account slot, ordered by id.
    pub fn accounts(&self) -> impl Iterator<Item = &AccountId> {
        self.sessions.keys()
    }

    /// The rendered account.
    pub fn active_account(&self) -> Option<&AccountId> {
        self.active.as_ref()
    }

    /// The rendered account's session; in live mode with no account
    /// rendered, the daemon session.
    pub fn session(&self) -> Option<&AccountSession> {
        self.active
            .as_ref()
            .and_then(|id| self.sessions.get(id))
            .or_else(|| self.live.as_ref().map(live::LiveLink::daemon))
    }

    /// The rendered session's scenario-playing source, if it has one (see
    /// [`AccountSession::mock_mut`]).
    pub fn mock_mut(&mut self) -> Option<&mut ramflux_desktop_test::MockSource> {
        let id = self.active.clone();
        match id {
            Some(id) => self.sessions.get_mut(&id)?.mock_mut(),
            None => None,
        }
    }

    /// The live daemon's own store (what the daemon scope knows, also while
    /// an account session is shown).
    pub fn daemon_store(&self) -> Option<&Store> {
        self.live.as_ref().map(|link| link.daemon().store())
    }

    /// Whether the live daemon's own session reports the daemon as not
    /// reachable (`false` without a live daemon).
    pub fn daemon_unreachable(&self) -> bool {
        self.live.as_ref().is_some_and(|link| {
            link.daemon().store().source_status()
                == ramflux_desktop_core::SourceStatus::Disconnected
        })
    }

    /// The source status the title bar shows. An account session that has
    /// received nothing yet (it just opened) is `Disconnected` only because
    /// it has not reported; the daemon's own session says whether the
    /// daemon is there, so that one is shown until the account has data.
    pub fn shown_source_status(&self) -> ramflux_desktop_core::SourceStatus {
        let own = self.store().source_status();
        let silent =
            self.store().accounts().next().is_none() && self.store().contexts().next().is_none();
        if own == ramflux_desktop_core::SourceStatus::Disconnected
            && self.active.is_some()
            && silent
            && let Some(daemon) = self.daemon_store()
        {
            return daemon.source_status();
        }
        own
    }

    /// Whether a live daemon feeds the handle.
    pub fn is_live(&self) -> bool {
        self.live.is_some()
    }

    /// Tells the rendered account's source which context the user looks at.
    pub fn focus(&mut self, context: Option<&ContextId>) {
        if let Some(session) = self.active_session_mut() {
            session.focus(context);
        }
    }

    /// Asks the rendered account's source for message bodies a view shows
    /// but the store's body cache no longer holds.
    pub fn request_bodies(&mut self, moments: &[MomentId]) {
        if let Some(session) = self.active_session_mut() {
            session.request_bodies(moments);
        }
    }

    /// The rendered account's store; an empty store before any account exists.
    pub fn store(&self) -> &Store {
        self.session().map_or(&EMPTY_STORE, AccountSession::store)
    }

    /// The store the entry page's ceremonies (creating an account with a
    /// recovery phrase, recovering one) read and are decided against: the
    /// daemon's own on a live daemon, also once the account a ceremony
    /// opened has a session of its own and is rendered (its store knows
    /// nothing of the ceremony that opened it); else the rendered store.
    pub fn entry_store(&self) -> &Store {
        self.daemon_store().unwrap_or_else(|| self.store())
    }

    /// Number of "needs me" moments in an account's store.
    pub fn needs_me_count(&self, account: &AccountId) -> usize {
        self.sessions.get(account).map_or(0, |s| selectors::attention(s.store()).len())
    }

    /// Display label of an account slot: the account record's label in its
    /// own store, or `None` while the slot has no account record yet.
    pub fn account_label(&self, account: &AccountId) -> Option<String> {
        self.sessions
            .get(account)
            .and_then(|s| s.store().account(account).or_else(|| s.store().accounts().next()))
            .map(|a| a.label.clone())
    }

    /// Every "needs me" item across accounts: the active account first, then
    /// the others in id order; oldest first within an account.
    pub fn attention_across_accounts(&self) -> Vec<AccountAttention> {
        let active = self.active.iter();
        let others = self.sessions.keys().filter(|id| Some(*id) != self.active.as_ref());
        active
            .chain(others)
            .filter_map(|account| self.sessions.get(account).map(|s| (account, s)))
            .flat_map(|(account, session)| {
                selectors::attention(session.store())
                    .into_iter()
                    .map(|item| AccountAttention { account: account.clone(), item })
            })
            .collect()
    }

    /// The inspector log.
    pub fn inspector(&self) -> &Inspector {
        &self.inspector
    }

    /// Appends an inspector entry recorded by a view or the root.
    pub fn record(&mut self, entry: InspectorEntry, cx: &mut Context<Self>) {
        self.inspector.push(entry);
        cx.emit(StoreEvent::Inspected);
    }

    /// Starts the real-time clock task. Calling it again restarts it.
    pub fn start_clock(&mut self, cx: &mut Context<Self>) {
        self.last_tick = Some(cx.background_executor().now());
        self.clock = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                let alive = this.update(cx, |handle, cx| handle.tick(cx)).is_ok();
                if !alive {
                    break;
                }
            }
        }));
    }

    /// Stops the clock task.
    pub fn stop_clock(&mut self) {
        self.clock = None;
        self.last_tick = None;
    }

    fn tick(&mut self, cx: &mut Context<Self>) {
        let now = cx.background_executor().now();
        let elapsed = self.last_tick.map_or(TICK, |last| now.saturating_duration_since(last));
        self.last_tick = Some(now);
        let ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
        self.advance(ms, cx);
    }

    /// Installs the audio calls use (the computer's devices in the
    /// application, a stand-in in tests). Without it calls show but carry
    /// no audio.
    pub fn set_call_audio(&mut self, audio: Rc<dyn CallAudio>) {
        self.call_audio = Some(audio);
    }

    /// The current call's audio, while a call is answered.
    pub fn call_audio(&self) -> Option<CallAudioView> {
        self.call.as_ref().map(call_audio::CallLink::view)
    }

    /// Mutes or unmutes the current call's microphone.
    pub fn set_call_muted(&mut self, muted: bool, cx: &mut Context<Self>) {
        if let Some(link) = self.call.as_mut() {
            link.set_muted(muted);
            cx.emit(StoreEvent::CallAudio);
        }
    }

    /// Makes the audio follow the calls: the ring tone while a call rings
    /// in the rendered account, the media engine while a call is under way.
    /// A call's audio stays with its account while another one is shown
    /// (its keys start once: stopping it would end its audio for good).
    fn drive_calls(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        let audio = self.call_audio.clone();
        let account =
            self.call.as_ref().map(|link| link.account.clone()).or_else(|| self.active.clone());
        let session = account.as_ref().and_then(|id| self.sessions.get_mut(id));
        let Some((account, session)) = account.zip(session) else {
            changed |= self.call.take().is_some();
            self.ringer = None;
            self.ringing = false;
            if changed {
                cx.emit(StoreEvent::CallAudio);
            }
            return;
        };
        while let Some((id, answer)) = session.take_call_media() {
            if let Some(link) =
                self.call.as_mut().filter(|l| l.call_id == id && l.account == account)
            {
                link.media(answer, audio.as_ref());
                changed = true;
            }
        }
        let (answered, active) = selectors::current_call(session.store())
            .filter(|c| matches!(c.state, CallState::Connecting | CallState::Active))
            .map_or((None, false), |c| (Some(c.id.clone()), c.state == CallState::Active));
        let linked = self.call.as_ref().map(|l| (&l.account, &l.call_id));
        if answered.as_ref().map(|id| (&account, id)) != linked {
            self.call = answered.map(|id| call_audio::CallLink::new(account.clone(), id));
            changed = true;
        }
        // The daemon hands the media session out once the call is active,
        // and only once: it is asked for once per call.
        if let Some(link) = self.call.as_mut().filter(|link| active && !link.requested) {
            link.requested = true;
            session.request_call_media(&link.call_id);
            changed = true;
        }
        let store = session.store();
        let ringing =
            !selectors::ringing_calls(store).is_empty() && selectors::current_call(store).is_none();
        if ringing != self.ringing {
            self.ringing = ringing;
            self.ringer = if ringing { audio.as_ref().and_then(|a| a.ring()) } else { None };
        }
        if changed {
            cx.emit(StoreEvent::CallAudio);
        }
    }

    /// Moves every unpaused session's clock forward by `ms`.
    pub fn advance(&mut self, ms: u64, cx: &mut Context<Self>) {
        let active = self.active.clone();
        let mut background = Vec::new();
        let mut foreground = None;
        for (account, session) in &mut self.sessions {
            let applied = session.advance(ms);
            if Some(account) == active.as_ref() {
                foreground = Some(applied);
            } else if !applied.changes.is_domain_empty() {
                background.push(account.clone());
            }
        }
        if let Some(applied) = foreground {
            self.publish(applied, cx);
        }
        for account in background {
            cx.emit(StoreEvent::BackgroundChanged(account));
        }
        self.advance_live(ms, cx);
        self.drive_calls(cx);
    }

    /// Whether the active session's clock is frozen.
    pub fn is_paused(&self) -> bool {
        self.session().is_some_and(AccountSession::is_paused)
    }

    /// Freezes or unfreezes the active session's clock.
    pub fn set_paused(&mut self, paused: bool, cx: &mut Context<Self>) {
        if let Some(session) = self.active_session_mut() {
            session.set_paused(paused);
            let mut changes = ChangeSet::default();
            changes.mark_clock();
            cx.emit(StoreEvent::Changed(Rc::new(changes)));
        }
    }

    /// Applies exactly the next scheduled envelope of the active session.
    pub fn step(&mut self, cx: &mut Context<Self>) {
        if let Some(applied) = self.active_session_mut().map(AccountSession::step) {
            self.publish(applied, cx);
        }
    }

    /// Resets the active store and replays its scenario up to `ms`.
    pub fn seek(&mut self, ms: u64, cx: &mut Context<Self>) {
        if let Some(applied) = self.active_session_mut().map(|s| s.seek(ms)) {
            self.publish(applied, cx);
        }
    }

    /// Runs policy for `intent` against the active store and submits the
    /// resulting effects to the active source.
    pub fn dispatch(&mut self, intent: Intent, cx: &mut Context<Self>) -> DispatchOutcome {
        if self.live.is_some() {
            return self.dispatch_live(intent, cx);
        }
        let Some(session) = self.active_session_mut() else {
            return DispatchOutcome::NoAccount;
        };
        let (outcome, records) = session.dispatch(intent);
        for record in records {
            self.inspector.push(record);
        }
        cx.emit(StoreEvent::Inspected);
        outcome
    }

    /// The recovery phrase the source handed out for the running creation
    /// ceremony, once. The entry page's ceremony runs in the daemon scope
    /// (no account is open); the caller owns the phrase and must drop it
    /// when the user hides it.
    pub fn take_recovery_phrase(&mut self) -> Option<ramflux_desktop_core::RecoveryPhrase> {
        // A live daemon's ceremonies run in the daemon's own session, also
        // once the account they open has a session of its own.
        if let Some(link) = self.live.as_mut() {
            return link.take_daemon_recovery_phrase();
        }
        self.active_session_mut()?.take_recovery_phrase()
    }

    fn active_session_mut(&mut self) -> Option<&mut AccountSession> {
        let id = self.active.clone()?;
        self.sessions.get_mut(&id)
    }

    fn publish(&mut self, applied: Applied, cx: &mut Context<Self>) {
        if applied.changes.is_empty() && applied.records.is_empty() {
            return;
        }
        for record in applied.records {
            self.inspector.push(record);
        }
        cx.emit(StoreEvent::Changed(Rc::new(applied.changes)));
    }

    fn emit_reset(&mut self, cx: &mut Context<Self>) {
        let mut changes = ChangeSet::reset();
        changes.mark_clock();
        cx.emit(StoreEvent::Changed(Rc::new(changes)));
    }
}

/// The current wall-clock time, for anchoring live playback.
pub(crate) fn wall_clock_now() -> Timestamp {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis());
    Timestamp(i64::try_from(millis).unwrap_or(i64::MAX))
}

/// The account a mock scenario plays: the first account it declares, or the
/// [`LOCAL_ACCOUNT`] slot.
pub fn scenario_account(scenario: &Scenario) -> AccountId {
    let timeline = scenario.timeline.iter().map(|(_, event)| event);
    let responses = scenario.responses.iter().flat_map(|rule| rule.emit.iter().map(|(_, e)| e));
    timeline
        .chain(responses)
        .find_map(|event| match event {
            DomainEvent::AccountUpdated(account) => Some(account.id.clone()),
            _ => None,
        })
        .unwrap_or_else(|| AccountId::new(LOCAL_ACCOUNT))
}

#[cfg(test)]
mod tests {
    use ramflux_desktop_test::load_fixture;

    use super::*;

    #[test]
    fn scenarios_map_to_their_declared_account() {
        let s01 = load_fixture("s01_first_launch").expect("s01");
        assert_eq!(scenario_account(&s01), AccountId::new("a-main"));
        let s02 = load_fixture("s02_daily").expect("s02");
        assert_eq!(scenario_account(&s02), AccountId::new(LOCAL_ACCOUNT));
        let s09 = load_fixture("s09_multi_account_day").expect("s09");
        assert_eq!(scenario_account(&s09), AccountId::new("a-personal"));
    }
}
