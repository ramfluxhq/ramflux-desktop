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

mod changes;
mod inspector;
mod session;

use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use gpui_kit::{Context, EventEmitter, Task};
use ramflux_desktop_core::{AccountId, DomainEvent, Intent, Store, selectors};
use ramflux_desktop_test::Scenario;

pub use changes::ChangeSet;
pub use inspector::{
    INSPECTOR_CAP, Inspector, InspectorEntry, UiOutcome, format_path, format_scope, intent_name,
    ui_event_name,
};
pub use session::{AccountSession, Applied, DispatchOutcome};

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
}

/// Owns every account session and is the only path to the store and source.
pub struct StoreHandle {
    sessions: BTreeMap<AccountId, AccountSession>,
    active: Option<AccountId>,
    inspector: Inspector,
    clock: Option<Task<()>>,
    last_tick: Option<Instant>,
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
        }
    }

    /// Plays `scenario` as the active account, replacing that account's session.
    pub fn load_scenario(&mut self, scenario: &Scenario, cx: &mut Context<Self>) {
        let account = scenario_account(scenario);
        self.sessions.insert(account.clone(), AccountSession::new(scenario));
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
        self.sessions.insert(account.clone(), AccountSession::new(scenario));
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

    /// The rendered account's session.
    pub fn session(&self) -> Option<&AccountSession> {
        self.active.as_ref().and_then(|id| self.sessions.get(id))
    }

    /// The rendered account's store; an empty store before any account exists.
    pub fn store(&self) -> &Store {
        self.session().map_or(&EMPTY_STORE, AccountSession::store)
    }

    /// Number of "needs me" moments in an account's store.
    pub fn needs_me_count(&self, account: &AccountId) -> usize {
        self.sessions.get(account).map_or(0, |s| selectors::attention(s.store()).len())
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
    }
}
