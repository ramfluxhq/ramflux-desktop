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
//! - `account.create.begin {display_name, principal_kind: "headless_ai",
//!   custodian_account}` begins an AI participant's ceremony as the
//!   custodian's session (the frame names the custodian): the custodian must
//!   be an unlocked, connected person's account, else the begin is refused
//!   `CustodySetupInvalid`, whatever the cause. Its `.confirm` takes the
//!   three words only (a passphrase is refused: the daemon makes the data
//!   passphrase) and the account is listed `kind: headless_ai`, unlocked.
//! - `account.export_agent_credentials {account, path}` writes a one-shot
//!   bundle (a file readable by its owner only, never replacing one) for the
//!   custodian, on a connection that attended the custodian's account
//!   (`subscription.open` with `attended_frontend`), and refuses as the
//!   daemon does otherwise; the bundle here holds no real credential.
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
    /// `human` or `headless_ai`.
    kind: &'static str,
    /// The account that custodies an AI participant.
    custodian: Option<String>,
    /// The credentials an export would write are still in the vault.
    provisioning: bool,
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
    /// The custodian of an AI participant's ceremony.
    custodian: Option<String>,
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
    /// The fields of the last confirm's body.
    last_confirm_fields: Vec<String>,
    /// The account each connection attended (`subscription.open` with
    /// `attended_frontend`).
    attended: BTreeMap<usize, String>,
    /// The listing names an AI participant's custodian
    /// (`custodian_account_id`), as a later daemon may.
    report_custodian: bool,
    /// Exports written: owner, AI participant, path.
    exports: Vec<(String, String, String)>,
    /// The daemon refuses every AI participant's begin, whatever the
    /// custodian (it checks something the desktop cannot see).
    refuse_headless: bool,
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

/// Writes the fake bundle: owner-only, never over a file that exists. It
/// holds the account id and a marker, no credential.
fn write_bundle(path: &str, agent: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    let mut file =
        std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)?;
    file.write_all(format!("{{\"account_id\":\"{agent}\",\"fake_bundle\":true}}").as_bytes())
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
                last_confirm_fields: Vec::new(),
                attended: BTreeMap::new(),
                report_custodian: false,
                exports: Vec::new(),
                refuse_headless: false,
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

    /// The fields the last creation confirm's body carried.
    pub fn last_confirm_fields(&self) -> Vec<String> {
        self.state().last_confirm_fields.clone()
    }

    /// The daemon refuses every AI participant's begin from now on.
    pub fn refuse_headless(&self, refuse: bool) {
        self.state().refuse_headless = refuse;
    }

    /// Whether the listing names an AI participant's custodian.
    pub fn report_custodian(&self, report: bool) {
        self.state().report_custodian = report;
    }

    /// The exports written, as `(owner, AI participant, path)`.
    pub fn exports(&self) -> Vec<(String, String, String)> {
        self.state().exports.clone()
    }

    /// Whether the credentials of `id` are still held (not exported yet).
    pub fn holds_credentials(&self, id: &str) -> bool {
        self.state().accounts.get(id).is_some_and(|account| account.provisioning)
    }

    /// The account `id` as listed: its kind and custodian.
    pub fn account_kind(&self, id: &str) -> Option<(&'static str, Option<String>)> {
        self.state().accounts.get(id).map(|account| (account.kind, account.custodian.clone()))
    }

    /// The connection attended `account` (a subscription as an attended
    /// frontend).
    pub fn attend(&self, connection: usize, account: &str) {
        self.state().attended.insert(connection, account.to_owned());
    }

    /// Adds an account the daemon lists: a person's (`headless_ai` false) or
    /// an AI participant custodied by `custodian`.
    pub fn add_account(&self, id: &str, label: &str, passphrase: &str, locked: bool) {
        let mut state = self.state();
        let networking = state.networking_for_new();
        state.accounts.insert(
            id.to_owned(),
            Account {
                label: label.to_owned(),
                passphrase: passphrase.to_owned(),
                anchor: format!("anchor-{id}"),
                locked,
                networking,
                kind: "human",
                custodian: None,
                provisioning: false,
            },
        );
    }

    /// Adds an AI participant `custodian` custodies, with its credentials
    /// still in the vault.
    pub fn add_agent(&self, id: &str, label: &str, custodian: &str) {
        self.add_account(id, label, "daemon-made", true);
        if let Some(account) = self.state().accounts.get_mut(id) {
            account.kind = "headless_ai";
            account.custodian = Some(custodian.to_owned());
            account.provisioning = true;
        }
    }

    /// The networking state of `id` (`live` or `networking_pending`).
    pub fn set_networking_of(&self, id: &str, live: bool) {
        if let Some(account) = self.state().accounts.get_mut(id) {
            account.networking = if live { "live" } else { "networking_pending" };
        }
    }

    /// Locks (or unlocks) `id`, as the daemon would.
    pub fn set_locked(&self, id: &str, locked: bool) {
        if let Some(account) = self.state().accounts.get_mut(id) {
            account.locked = locked;
        }
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
                kind: "human",
                custodian: None,
                provisioning: false,
            },
        );
    }

    /// Whether this fake serves `account.list` (it owns accounts).
    pub fn serves_accounts(&self) -> bool {
        !self.state().accounts.is_empty()
    }

    /// The connection ended: its ceremony is erased with it.
    pub fn disconnect(&self, connection: usize) {
        let mut state = self.state();
        state.ceremonies.remove(&connection);
        state.attended.remove(&connection);
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
            "account.export_agent_credentials" => self.serves_accounts(),
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
            "account.create" | "account.create.begin" => {
                Self::begin(&mut state, connection, account, body)
            }
            "account.export_agent_credentials" => {
                Self::export(&mut state, connection, account, body)
            }
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
                let mut row = json!({ "id": id, "label": a.label, "kind": a.kind,
                                      "state": if a.locked { "locked" } else { "unlocked" },
                                      "networking": a.networking });
                if state.report_custodian
                    && let Some(custodian) = &a.custodian
                {
                    row["custodian_account_id"] = json!(custodian);
                }
                row
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

    /// The custodian an AI participant's begin names, when the daemon
    /// accepts it: the frame is the custodian's own session, and the
    /// custodian is an unlocked, connected person's account.
    fn custodian_of_begin(state: &State, account: Option<&str>, body: &Value) -> Option<String> {
        let custodian = body["custodian_account"].as_str()?;
        let found = state.accounts.get(custodian)?;
        let accepted = account == Some(custodian)
            && found.kind == "human"
            && !found.locked
            && found.networking == "live";
        accepted.then(|| custodian.to_owned())
    }

    fn begin(state: &mut State, connection: usize, account: Option<&str>, body: &Value) -> Reply {
        state.ceremonies.remove(&connection);
        if !state.vault_available {
            return refuse("CustodyVaultUnavailable");
        }
        let label = body["display_name"].as_str().unwrap_or_default();
        let custodian = match body["principal_kind"].as_str() {
            Some("human") if body.get("custodian_account").is_none() => None,
            Some("headless_ai") if state.refuse_headless => return refuse("CustodySetupInvalid"),
            Some("headless_ai") => match Self::custodian_of_begin(state, account, body) {
                Some(custodian) => Some(custodian),
                None => return refuse("CustodySetupInvalid"),
            },
            _ => return refuse("CustodySetupInvalid"),
        };
        if label.is_empty() {
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
                custodian,
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
        state.last_confirm_fields =
            body.as_object().map(|map| map.keys().cloned().collect()).unwrap_or_default();
        let agent = c.custodian.is_some();
        // An AI participant's data passphrase is the daemon's to make: a
        // confirm that brings one is refused.
        if agent && body.get("passphrase").is_some() {
            return refuse("CustodySetupInvalid");
        }
        let words: Vec<String> = body["words"]
            .as_array()
            .map(|w| w.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
            .unwrap_or_default();
        let passphrase = if agent {
            "daemon-made".to_owned()
        } else {
            body["passphrase"].as_str().unwrap_or_default().to_owned()
        };
        state.last_confirm =
            Some((words.clone(), body["passphrase"].as_str().unwrap_or_default().to_owned()));
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
        let id = format!("acct-{}-{}", if agent { "agent" } else { "native" }, state.began);
        let networking = state.networking_for_new();
        state.accounts.insert(
            id.clone(),
            Account {
                label: c.label.clone(),
                passphrase,
                anchor: format!("anchor-{id}"),
                locked: false,
                networking,
                kind: if agent { "headless_ai" } else { "human" },
                custodian: c.custodian.clone(),
                provisioning: agent,
            },
        );
        Reply::Ok(json!({ "account_id": id, "display_name": c.label, "state": "unlocked",
                          "networking": networking }))
    }

    /// `account.export_agent_credentials`, as the daemon takes it: from the
    /// custodian's attended session only, never replacing a file, and once.
    fn export(state: &mut State, connection: usize, account: Option<&str>, body: &Value) -> Reply {
        let permission = || {
            Reply::Error("ValidationFailed", "local bus peer credential check failed".to_owned())
        };
        let Some(owner) = account else { return permission() };
        let open = state.accounts.get(owner).is_some_and(|found| !found.locked);
        if state.attended.get(&connection).map(String::as_str) != Some(owner) || !open {
            return permission();
        }
        let fields: Vec<&str> = body
            .as_object()
            .map(|map| map.keys().map(String::as_str).collect())
            .unwrap_or_default();
        let (Some(agent), Some(path), 2) =
            (body["account"].as_str(), body["path"].as_str(), fields.len())
        else {
            return Reply::Error("ValidationFailed", "unknown field".to_owned());
        };
        let connected = |id: &str, state: &State| {
            state.accounts.get(id).is_some_and(|found| found.networking == "live")
        };
        if state.accounts.get(owner).is_none_or(|found| found.kind != "human") {
            return refuse("CustodySetupInvalid");
        }
        if !connected(owner, state) {
            return refuse("NetworkingPending");
        }
        let Some(found) = state.accounts.get(agent) else { return refuse("CustodySetupInvalid") };
        if found.kind != "headless_ai" || found.custodian.as_deref() != Some(owner) {
            return refuse("CustodySetupInvalid");
        }
        if !connected(agent, state) {
            return refuse("NetworkingPending");
        }
        if !found.provisioning {
            return refuse("CustodyVaultUnavailable");
        }
        match write_bundle(path, agent) {
            Ok(()) => {}
            Err(error) => return Reply::Error("ValidationFailed", format!("io error: {error}")),
        }
        if let Some(found) = state.accounts.get_mut(agent) {
            found.provisioning = false;
        }
        state.exports.push((owner.to_owned(), agent.to_owned(), path.to_owned()));
        Reply::Ok(json!({ "account": agent, "path": path, "exported": true }))
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
