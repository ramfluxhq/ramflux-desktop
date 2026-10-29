// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The daemon side of the born-v3 creation and recovery ceremonies on the
//! fake local bus, following the bus contract of the product revision the
//! desktop is pinned to (`docs/local_bus_account_creation.md`):
//!
//! - `account.create {display_name, principal_kind}` begins and answers
//!   `{ceremony, ceremony_id, expires_in_ms}`; `.show_once` hands the phrase
//!   out once; `.hidden_challenge` chooses three positions; `.confirm
//!   {words, passphrase}` is the single attempt and answers the account;
//!   `.cancel` is idempotent.
//! - `account.recover.begin {account_id}` answers the account fingerprint;
//!   `.confirm {phrase, passphrase, confirmed_anchor}` checks all three;
//!   `.cancel`.
//! - A ceremony belongs to the connection it began on: another connection
//!   cannot continue it, and the end of the connection erases it.
//! - Accounts it made survive a restart of the fake bus (give the same
//!   [`FakeCustody`] to the next one), are listed locked or unlocked with
//!   their networking state, and unlock with the passphrase set at confirm.
//!
//! The phrase is the mock's fixed non-production sequence of list words
//! ([`crate::mock_ceremony::demo_word`]); never a real phrase.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::fake_bus::Reply;
use crate::mock_ceremony::{DEMO_POSITIONS, demo_word};

/// The fixed fake phrase.
pub fn phrase() -> String {
    (1..=24).map(demo_word).collect::<Vec<_>>().join(" ")
}

#[derive(Clone, Debug)]
struct Account {
    label: String,
    passphrase: String,
    anchor: String,
    locked: bool,
    networking: &'static str,
}

#[derive(Debug)]
enum Step {
    Prepared,
    Shown,
    Challenged,
}

#[derive(Debug)]
struct Creation {
    label: String,
    step: Step,
    deadline: Instant,
}

#[derive(Debug)]
struct Recovery {
    account_id: String,
    anchor: String,
    deadline: Instant,
}

impl State {
    /// What a new account reports: forced, else live only with a default node.
    fn networking_for_new(&self) -> &'static str {
        self.networking_override.unwrap_or(if self.default_node.is_some() {
            "live"
        } else {
            "networking_pending"
        })
    }
}

#[derive(Debug)]
enum Ceremony {
    Creation(Creation),
    Recovery(Recovery),
}

#[derive(Debug)]
struct State {
    ttl: Duration,
    vault_available: bool,
    /// Forces the networking state of accounts made from now on.
    networking_override: Option<&'static str>,
    /// The daemon's default node (`node.set_default`): as in the product, an
    /// account made while there is one goes live inside confirm, else it
    /// stays `networking_pending`.
    default_node: Option<String>,
    positions: [u8; 3],
    began: u32,
    ceremonies: BTreeMap<usize, Ceremony>,
    accounts: BTreeMap<String, Account>,
    /// Methods seen, in order, as `(connection, method)`.
    log: Vec<(usize, String)>,
    /// The passphrases and words received by the last confirm (tests read
    /// them to prove what was sent).
    last_confirm: Option<(Vec<String>, String)>,
}

/// The fake daemon's custody state, shared across restarts of the fake bus.
#[derive(Debug)]
pub struct FakeCustody {
    state: Mutex<State>,
    /// Whether it answers anything (a fake daemon with its own accounts
    /// leaves the methods to its handler).
    enabled: bool,
}

impl Default for FakeCustody {
    fn default() -> Self {
        Self::new()
    }
}

fn refuse(code: &'static str) -> Reply {
    Reply::Error(code, code.to_owned())
}

impl FakeCustody {
    /// A daemon with no accounts, a five-minute deadline and a usable vault.
    pub fn new() -> Self {
        Self::with_enabled(true)
    }

    /// A custody that answers nothing: the fake's handler has the methods.
    pub fn disabled() -> Self {
        Self::with_enabled(false)
    }

    fn with_enabled(enabled: bool) -> Self {
        Self {
            enabled,
            state: Mutex::new(State {
                ttl: Duration::from_secs(300),
                vault_available: true,
                networking_override: None,
                default_node: None,
                positions: DEMO_POSITIONS,
                began: 0,
                ceremonies: BTreeMap::new(),
                accounts: BTreeMap::new(),
                log: Vec::new(),
                last_confirm: None,
            }),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().expect("custody")
    }

    /// How long a ceremony lives from now on.
    pub fn set_ttl(&self, ttl: Duration) {
        self.state().ttl = ttl;
    }

    /// Whether the primary vault is usable (begin refuses when not).
    pub fn set_vault_available(&self, available: bool) {
        self.state().vault_available = available;
    }

    /// The networking state of accounts made from now on.
    pub fn set_live(&self, live: bool) {
        self.state().networking_override = Some(if live { "live" } else { "networking_pending" });
    }

    /// The default node the daemon holds (what `node.set_default` set).
    pub fn default_node(&self) -> Option<String> {
        self.state().default_node.clone()
    }

    /// The account's networking comes up (registration succeeded later).
    pub fn go_live(&self, id: &str) {
        if let Some(account) = self.state().accounts.get_mut(id) {
            account.networking = "live";
        }
    }

    /// Ceremonies running now (one per connection at most).
    pub fn running(&self) -> usize {
        self.state().ceremonies.len()
    }

    /// The connections that have seen `method`.
    pub fn connections_of(&self, method: &str) -> Vec<usize> {
        let state = self.state();
        state.log.iter().filter(|(_, m)| m == method).map(|(c, _)| *c).collect()
    }

    /// The words and passphrase the last creation confirm carried.
    pub fn last_confirm(&self) -> Option<(Vec<String>, String)> {
        self.state().last_confirm.clone()
    }

    /// The account fingerprint the daemon names for `id`.
    pub fn anchor_of(&self, id: &str) -> Option<String> {
        self.state().accounts.get(id).map(|account| account.anchor.clone())
    }

    /// Locks every account, as a restart of the daemon does.
    pub fn restart(&self) {
        let mut state = self.state();
        state.ceremonies.clear();
        for account in state.accounts.values_mut() {
            account.locked = true;
        }
    }

    /// Adds a locked account whose credential is gone from this device
    /// (what recovery restores).
    pub fn add_locked_account(&self, id: &str, label: &str, passphrase: &str) {
        let mut state = self.state();
        let anchor = format!("anchor-{id}");
        let networking = state.networking_for_new();
        state.accounts.insert(
            id.to_owned(),
            Account {
                label: label.to_owned(),
                passphrase: passphrase.to_owned(),
                anchor,
                locked: true,
                networking,
            },
        );
    }

    /// Whether this fake serves `account.list` (it owns accounts).
    pub fn serves_accounts(&self) -> bool {
        !self.state().accounts.is_empty()
    }

    /// The connection ended: its ceremony is erased with it.
    pub fn disconnect(&self, connection: usize) {
        self.state().ceremonies.remove(&connection);
    }

    /// Whether `method` is one this fake answers.
    pub fn handles(&self, method: &str, account: Option<&str>, body: &Value) -> bool {
        if !self.enabled {
            return false;
        }
        match method {
            // Begin is `account.create` without the one-shot create's fields.
            "account.create" => body.get("display_name").is_some() && body.get("node_id").is_none(),
            "node.set_default" => true,
            "account.list" | "account.lock" | "account.unlock" => self.serves_accounts(),
            "account.status" => account.is_some_and(|id| self.state().accounts.contains_key(id)),
            _ => method.starts_with("account.create.") || method.starts_with("account.recover."),
        }
    }

    /// Answers a request this fake handles.
    pub fn answer(
        &self,
        connection: usize,
        method: &str,
        account: Option<&str>,
        body: &Value,
    ) -> Reply {
        let mut state = self.state();
        state.log.push((connection, method.to_owned()));
        match method {
            "node.set_default" => {
                state.default_node = body["node_id"].as_str().map(str::to_owned);
                Reply::Ok(json!({ "node_id": body["node_id"], "default": true }))
            }
            "account.status" => Self::status(&state, account),
            "account.list" => Self::list(&state),
            "account.lock" => Self::lock(&mut state, account),
            "account.unlock" => Self::unlock(&mut state, account, body),
            "account.create" => Self::begin(&mut state, connection, body),
            "account.recover.begin" => Self::recover_begin(&mut state, connection, body),
            "account.create.cancel" | "account.recover.cancel" => {
                state.ceremonies.remove(&connection);
                Reply::Ok(json!({ "cancelled": true }))
            }
            _ => Self::step(&mut state, connection, method, body),
        }
    }

    fn status(state: &State, account: Option<&str>) -> Reply {
        let Some((id, found)) = account.and_then(|id| state.accounts.get(id).map(|a| (id, a)))
        else {
            return refuse("AccountNotOpen");
        };
        if found.locked {
            return Reply::Error("AccountNotOpen", format!("account not open: {id}"));
        }
        Reply::Ok(json!({ "account_id": id, "local_account_id": id, "state": "unlocked",
                          "networking": found.networking, "display_name": found.label,
                          "principal_id": format!("principal-{id}"), "device_id": format!("device-{id}"),
                          "node_id": state.default_node, "principal_commitment": found.anchor }))
    }

    fn list(state: &State) -> Reply {
        let open: Vec<&String> =
            state.accounts.iter().filter(|(_, a)| !a.locked).map(|(id, _)| id).collect();
        let details: Vec<Value> = state
            .accounts
            .iter()
            .map(|(id, a)| {
                json!({ "id": id, "label": a.label, "kind": "human",
                        "state": if a.locked { "locked" } else { "unlocked" },
                        "networking": a.networking })
            })
            .collect();
        Reply::Ok(
            json!({ "accounts": open, "account_details": details, "active_account_id": null }),
        )
    }

    fn lock(state: &mut State, account: Option<&str>) -> Reply {
        match account.and_then(|id| state.accounts.get_mut(id)) {
            Some(found) => {
                found.locked = true;
                Reply::Ok(json!({ "state": "locked" }))
            }
            None => refuse("AccountNotOpen"),
        }
    }

    fn unlock(state: &mut State, account: Option<&str>, body: &Value) -> Reply {
        let Some(id) = account else { return refuse("AccountNotOpen") };
        let Some(found) = state.accounts.get_mut(id) else { return refuse("AccountNotOpen") };
        if body["passphrase"].as_str() != Some(found.passphrase.as_str()) {
            return Reply::Error("WrongPassphrase", "wrong passphrase".to_owned());
        }
        found.locked = false;
        Reply::Ok(json!({ "account_id": id, "local_account_id": id, "state": "unlocked",
                          "networking": found.networking }))
    }

    fn expired(ceremony: &Ceremony) -> bool {
        let deadline = match ceremony {
            Ceremony::Creation(c) => c.deadline,
            Ceremony::Recovery(r) => r.deadline,
        };
        Instant::now() >= deadline
    }

    fn begin(state: &mut State, connection: usize, body: &Value) -> Reply {
        state.ceremonies.remove(&connection);
        if !state.vault_available {
            return refuse("CustodyVaultUnavailable");
        }
        let label = body["display_name"].as_str().unwrap_or_default();
        if label.is_empty() || body["principal_kind"] != "human" {
            return refuse("CustodySetupInvalid");
        }
        state.began += 1;
        let deadline = Instant::now() + state.ttl;
        state.ceremonies.insert(
            connection,
            Ceremony::Creation(Creation {
                label: label.to_owned(),
                step: Step::Prepared,
                deadline,
            }),
        );
        Reply::Ok(
            json!({ "ceremony": "pending", "ceremony_id": format!("ceremony-{}", state.began),
                          "expires_in_ms": u64::try_from(state.ttl.as_millis()).unwrap_or(0) }),
        )
    }

    fn recover_begin(state: &mut State, connection: usize, body: &Value) -> Reply {
        state.ceremonies.remove(&connection);
        if !state.vault_available {
            return refuse("CustodyVaultUnavailable");
        }
        let id = body["account_id"].as_str().unwrap_or_default();
        let Some(account) = state.accounts.get(id) else { return refuse("CustodySetupInvalid") };
        let anchor = account.anchor.clone();
        state.began += 1;
        let deadline = Instant::now() + state.ttl;
        state.ceremonies.insert(
            connection,
            Ceremony::Recovery(Recovery {
                account_id: id.to_owned(),
                anchor: anchor.clone(),
                deadline,
            }),
        );
        Reply::Ok(json!({
            "ceremony_id": format!("recovery-{}", state.began),
            "account_anchor": anchor,
            "expires_in_ms": u64::try_from(state.ttl.as_millis()).unwrap_or(0),
            "warning": "Confirm this account fingerprint.",
        }))
    }

    /// Every other ceremony step. A refusal erases the ceremony.
    fn step(state: &mut State, connection: usize, method: &str, body: &Value) -> Reply {
        let Some(ceremony) = state.ceremonies.remove(&connection) else {
            return refuse("CustodySetupInvalid");
        };
        if Self::expired(&ceremony) {
            return refuse("CustodyExpired");
        }
        match (method, ceremony) {
            ("account.create.show_once", Ceremony::Creation(mut c))
                if matches!(c.step, Step::Prepared) =>
            {
                c.step = Step::Shown;
                state.ceremonies.insert(connection, Ceremony::Creation(c));
                Reply::Ok(json!({ "phrase": phrase() }))
            }
            ("account.create.hidden_challenge", Ceremony::Creation(mut c))
                if matches!(c.step, Step::Shown) =>
            {
                c.step = Step::Challenged;
                state.ceremonies.insert(connection, Ceremony::Creation(c));
                Reply::Ok(json!({ "positions": state.positions }))
            }
            ("account.create.confirm", Ceremony::Creation(c))
                if matches!(c.step, Step::Challenged) =>
            {
                Self::confirm_creation(state, &c, body)
            }
            ("account.recover.confirm", Ceremony::Recovery(r)) => {
                Self::confirm_recovery(state, &r, body)
            }
            _ => refuse("CustodySetupInvalid"),
        }
    }

    fn confirm_creation(state: &mut State, c: &Creation, body: &Value) -> Reply {
        let words: Vec<String> = body["words"]
            .as_array()
            .map(|w| w.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
            .unwrap_or_default();
        let passphrase = body["passphrase"].as_str().unwrap_or_default().to_owned();
        state.last_confirm = Some((words.clone(), passphrase.clone()));
        if passphrase.is_empty() || words.len() != 3 {
            return refuse("CustodySetupInvalid");
        }
        let right = state
            .positions
            .iter()
            .zip(&words)
            .all(|(position, word)| word == demo_word(usize::from(*position)));
        if !right {
            return refuse("CustodyConfirmationFailed");
        }
        let id = format!("acct-native-{}", state.began);
        let networking = state.networking_for_new();
        state.accounts.insert(
            id.clone(),
            Account {
                label: c.label.clone(),
                passphrase,
                anchor: format!("anchor-{id}"),
                locked: false,
                networking,
            },
        );
        Reply::Ok(json!({ "account_id": id, "display_name": c.label, "state": "unlocked",
                          "networking": networking }))
    }

    fn confirm_recovery(state: &mut State, r: &Recovery, body: &Value) -> Reply {
        if body["confirmed_anchor"].as_str() != Some(r.anchor.as_str())
            || body["phrase"].as_str() != Some(phrase().as_str())
        {
            return refuse("CustodyConfirmationFailed");
        }
        let Some(account) = state.accounts.get_mut(&r.account_id) else {
            return refuse("CustodySetupInvalid");
        };
        // The account's own data passphrase is checked before anything changes.
        if body["passphrase"].as_str() != Some(account.passphrase.as_str()) {
            return refuse("CustodySetupInvalid");
        }
        account.locked = false;
        Reply::Ok(json!({ "account_id": r.account_id, "display_name": account.label,
                          "state": "unlocked", "networking": account.networking }))
    }
}
