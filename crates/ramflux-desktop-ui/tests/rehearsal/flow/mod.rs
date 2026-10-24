// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The tier ② demo, step by step, on two desktops and two daemons:
//!
//! 1. both desktops attach to their daemons;
//! 2. each adds the node by URL, checks the shown key fingerprint against
//!    the expected key and confirms it with the hold;
//! 3. each creates its account (Alice, Bob) on the node;
//! 4. each adds the other as a contact and verifies the safety number
//!    where the daemon offers it;
//! 5. direct messages both ways, with receipts;
//! 6. Alice sends Bob an attachment;
//! 7. Alice locks and unlocks, her daemon restarts, the history stays.
//!
//! Each step asserts the resulting store state and takes light and dark
//! screenshots. Steps live in `setup` (1 to 3), `talk` (4 to 6) and
//! `resume` (7); `probe` reads the stores.

mod evidence;
mod group;
mod probe;
mod reopen;
mod resume;
mod setup;
mod talk;

use std::path::{Path, PathBuf};

use ramflux_desktop_core::{ContextId, NodeId};
use ramflux_desktop_ui::shell::ShellOptions;

use super::config::{self, DemoConfig};
use super::daemon::DemoDaemon;
use super::desk::Desk;
use super::host::Host;
use super::steps::{Notes, Steps};

/// What one side learned about its account in step 3.
#[derive(Debug, Clone)]
pub struct Facts {
    /// Local account id.
    pub account: String,
    /// Principal (the participant id contacts use).
    pub principal: String,
    /// This device.
    pub device: String,
    /// Principal commitment (from the reference CLI's account status).
    pub commitment: String,
    /// Delivery target.
    pub target: String,
}

/// One person: a daemon, a desktop and an account.
pub struct Side<D> {
    /// Short tag for files and messages ("a", "b").
    pub tag: &'static str,
    /// Who this is in notes and messages ("Alice", "Bob").
    pub name: &'static str,
    /// The account label typed at account creation (the name by default; a
    /// live run on a shared node uses a marked test label).
    pub label: String,
    /// The daemon.
    pub daemon: D,
    /// The desktop, once opened.
    pub desk: Option<Desk>,
    /// The account passphrase (demo data; never printed).
    passphrase: String,
    /// The account, once created.
    pub facts: Option<Facts>,
}

impl<D> Side<D> {
    /// A side named `name` on `daemon`.
    pub fn new(tag: &'static str, name: &'static str, daemon: D) -> Self {
        Self {
            tag,
            name,
            label: name.to_owned(),
            daemon,
            desk: None,
            passphrase: config::passphrase(),
            facts: None,
        }
    }

    /// The same side with the account label `label`.
    #[must_use]
    pub fn labelled(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    fn desk(&self) -> Result<&Desk, String> {
        self.desk.as_ref().ok_or_else(|| format!("desktop {} is not open", self.tag))
    }

    fn facts(&self) -> Result<&Facts, String> {
        self.facts.as_ref().ok_or_else(|| format!("account {} was not created", self.name))
    }
}

/// Builds the shell options of a desktop attached to a socket.
pub type OptionsFor = Box<dyn Fn(&Path) -> ShellOptions>;

/// A rehearsal in progress.
pub struct Demo<'h, H: Host, D: DemoDaemon> {
    host: &'h mut H,
    config: DemoConfig,
    options: OptionsFor,
    sides: [Side<D>; 2],
    work: PathBuf,
    node: Option<NodeId>,
    /// The direct conversation Alice starts in step 5 (empty until then).
    conversation: ContextId,
    texts: Vec<String>,
    /// The group conversation of the live group step and its texts.
    group: Option<(ContextId, Vec<String>)>,
    /// Where the reference CLI had to receive for a desktop (the stream did
    /// not deliver in time).
    fallbacks: Vec<String>,
}

impl<'h, H: Host, D: DemoDaemon> Demo<'h, H, D> {
    /// A rehearsal of `alice` and `bob` on `host`. `work` is a scratch
    /// folder for the attachment; the caller removes it.
    pub fn new(
        host: &'h mut H,
        config: DemoConfig,
        options: OptionsFor,
        alice: Side<D>,
        bob: Side<D>,
        work: PathBuf,
    ) -> Self {
        let conversation = ContextId::new("");
        Self {
            host,
            config,
            options,
            sides: [alice, bob],
            work,
            node: None,
            conversation,
            texts: Vec::new(),
            group: None,
            fallbacks: Vec::new(),
        }
    }

    /// The two sides (Alice, Bob).
    pub fn sides(&self) -> &[Side<D>; 2] {
        &self.sides
    }

    /// The node confirmed in step 2.
    pub fn node(&self) -> Option<&NodeId> {
        self.node.as_ref()
    }

    /// The direct conversation of step 5.
    pub fn conversation(&self) -> &ContextId {
        &self.conversation
    }

    /// The rehearsal's screenshot folder.
    pub fn shots_dir(&self) -> Option<&Path> {
        self.config.shots.as_deref()
    }

    /// Ticks until `done` holds on side `ix`; a timeout names the store's
    /// last failures.
    fn wait(
        &mut self,
        ix: usize,
        limit: std::time::Duration,
        what: &str,
        done: impl FnMut(&Desk, &mut H) -> bool,
    ) -> Result<std::time::Duration, String> {
        let desk = self.sides[ix].desk()?;
        desk.until(self.host, limit, what, done).map_err(|error| {
            let failures = desk.store(self.host, probe::failures);
            format!("{error}{failures}")
        })
    }

    /// Reads side `ix`'s store.
    fn read<R>(
        &mut self,
        ix: usize,
        f: impl FnOnce(&ramflux_desktop_core::Store) -> R,
    ) -> Result<R, String> {
        let desk = self.sides[ix].desk()?;
        Ok(desk.store(self.host, f))
    }

    /// Takes light and dark screenshots of side `ix` as `name`.
    fn shots(&mut self, ix: usize, name: &str, notes: &mut Notes) -> Result<(), String> {
        let Some(dir) = self.config.shots.clone() else { return Ok(()) };
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        let desk = self.sides[ix].desk()?;
        let written = desk.shots(self.host, &dir, name)?;
        if written > 0 {
            notes.add(format!("{name}_{}_{{light,dark}}.png", desk.tag));
        }
        Ok(())
    }
}

/// Runs every step and returns the table.
pub fn run<H: Host, D: DemoDaemon>(demo: &mut Demo<'_, H, D>) -> Steps {
    let mut steps = Steps::default();
    steps.run(1, "Two desktops attach to two daemons", &[], |n| setup::start(demo, n));
    steps.run(2, "Add the node by URL, check its key, confirm with the hold", &[1], |n| {
        setup::node(demo, n)
    });
    steps.run(3, "Create Alice and Bob on the node", &[2], |n| setup::accounts(demo, n));
    steps.run(4, "Add each other as contacts; verify safety numbers", &[3], |n| {
        talk::contacts(demo, n)
    });
    steps.run(5, "Direct messages both ways, with receipts", &[4], |n| talk::messages(demo, n));
    steps.run(6, "Alice sends Bob an attachment", &[5], |n| talk::attachment(demo, n));
    steps.run(7, "Lock and unlock Alice; restart her daemon; history kept", &[5], |n| {
        resume::lock_and_restart(demo, n)
    });
    steps
}

/// Runs the live rehearsal: steps 1 to 6 as [`run`], then the steps a real
/// node adds (7: a group made on Alice's daemon, messages both ways in it
/// from the composers; 8: Bob's desktop quits and reopens and resumes from
/// its stream cursor; 9: what the frame taps saw, polling off), then
/// Alice's lock and daemon restart as step 10.
pub fn run_live<H: Host, D: DemoDaemon>(demo: &mut Demo<'_, H, D>) -> Steps {
    let mut steps = Steps::default();
    steps.run(1, "Two desktops attach to two daemons", &[], |n| setup::start(demo, n));
    steps.run(2, "Add the node by URL, check its key, confirm with the hold", &[1], |n| {
        setup::node(demo, n)
    });
    steps.run(3, "Create Alice and Bob on the node", &[2], |n| setup::accounts(demo, n));
    steps.run(4, "Add each other as contacts; verify safety numbers", &[3], |n| {
        talk::contacts(demo, n)
    });
    steps.run(5, "Direct messages both ways, with receipts", &[4], |n| talk::messages(demo, n));
    steps.run(6, "Alice sends Bob an attachment", &[5], |n| talk::attachment(demo, n));
    steps.run(7, "A group with both; messages both ways in it", &[4], |n| group::group(demo, n));
    steps.run(8, "Bob's desktop quits and reopens; resumes from its cursor", &[5], |n| {
        reopen::reopen(demo, n)
    });
    steps.run(9, "Live receive on the event stream, polling off (frame taps)", &[5], |n| {
        evidence::stream(demo, n)
    });
    steps.run(10, "Lock and unlock Alice; restart her daemon; history kept", &[5], |n| {
        resume::lock_and_restart(demo, n)
    });
    steps
}
