// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! What a rehearsal runs against: the node URL, the node key the operator
//! expects, the run id and where screenshots go. Read from the environment
//! for a live run; built directly by the fake-daemon tests.

use std::path::PathBuf;
use std::time::Duration;

/// The switch that enables the live rehearsal.
pub const ENABLE_ENV: &str = "RAMFLUX_DEMO_REHEARSAL";
/// The node's discovery URL.
pub const NODE_URL_ENV: &str = "RAMFLUX_DEMO_NODE_URL";
/// The node's public key as the operator published it (or its fingerprint).
pub const NODE_KEY_ENV: &str = "RAMFLUX_DEMO_NODE_KEY";
/// The folder the run's screenshot folder is made in.
pub const SHOTS_ENV: &str = "RAMFLUX_DEMO_SHOTS_DIR";

/// How long the rehearsal waits for the daemon or the node.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// For a daemon or node answer the step needs.
    pub answer: Duration,
    /// For a message to arrive by the desktop's own polling before the
    /// reference CLI is asked to receive it.
    pub discover: Duration,
    /// For a delivery receipt.
    pub receipt: Duration,
    /// For a safety number to become verifiable and be verified.
    pub verify: Duration,
}

impl Limits {
    /// Limits for a real node over the network.
    pub const LIVE: Self = Self {
        answer: Duration::from_secs(90),
        discover: Duration::from_secs(20),
        receipt: Duration::from_secs(60),
        verify: Duration::from_secs(30),
    };

    /// Limits for the fake daemon (they only bound a failure).
    pub const FAKE: Self = Self {
        answer: Duration::from_secs(30),
        discover: Duration::from_secs(2),
        receipt: Duration::from_secs(30),
        verify: Duration::from_secs(10),
    };
}

/// One rehearsal's settings.
#[derive(Debug, Clone)]
pub struct DemoConfig {
    /// The node's discovery URL.
    pub node_url: String,
    /// The node key the operator expects (public key or fingerprint).
    pub node_key: String,
    /// The run id (screenshot folder name, conversation id suffix).
    pub run_id: String,
    /// Where this run's screenshots go; `None` takes none.
    pub shots: Option<PathBuf>,
    /// Wait limits.
    pub limits: Limits,
}

impl DemoConfig {
    /// The live settings from the environment: `Ok(None)` when the
    /// rehearsal is not switched on, an error naming what is missing.
    ///
    /// # Errors
    ///
    /// Returns which variable is missing when the switch is on.
    pub fn from_env(default_shots: PathBuf) -> Result<Option<Self>, String> {
        if std::env::var(ENABLE_ENV).ok().as_deref() != Some("1") {
            return Ok(None);
        }
        let var = |name: &str| {
            std::env::var(name)
                .ok()
                .map(|v| v.trim().to_owned())
                .filter(|v| !v.is_empty())
                .ok_or_else(|| format!("{name} is not set"))
        };
        let run_id = run_id();
        let base = std::env::var_os(SHOTS_ENV).map_or(default_shots, PathBuf::from);
        Ok(Some(Self {
            node_url: var(NODE_URL_ENV)?,
            node_key: var(NODE_KEY_ENV)?,
            shots: Some(base.join(&run_id)),
            run_id,
            limits: Limits::LIVE,
        }))
    }
}

/// A run id from the local time, e.g. `20261023-142501`.
pub fn run_id() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
}

/// The fingerprint the desktop shows for a node public key (the product's
/// client-discovery hash of the key as published).
pub fn fingerprint_of(public_key: &str) -> String {
    ramflux_protocol::hash_hex(ramflux_protocol::CLIENT_DISCOVERY_DOMAIN, public_key.as_bytes())
}

/// Whether the fingerprint the desktop shows belongs to the expected key.
/// The operator may hand over the public key itself or its fingerprint.
pub fn fingerprint_matches(expected: &str, shown: &str) -> bool {
    let expected = expected.trim();
    !shown.is_empty() && (shown == fingerprint_of(expected) || shown.eq_ignore_ascii_case(expected))
}

/// A fresh random passphrase for a demo account (never printed).
pub fn passphrase() -> String {
    use std::io::Read as _;
    let mut bytes = [0_u8; 18];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .expect("system randomness");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
