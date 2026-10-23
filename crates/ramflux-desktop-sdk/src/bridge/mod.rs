// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The bridge thread: the only place a tokio runtime runs.
//!
//! The GPUI side talks to it through two runtime-agnostic channels: commands
//! go in through an unbounded sender (a plain, non-blocking `send`), and each
//! source drains its own `std::sync::mpsc` outbox. Every scope (the daemon,
//! each unlocked account) runs as its own task with its own request connection,
//! subscription and poll timer, so scopes never wait on one another.
//! A graceful daemon stop runs on a connection of its own.
//!
//! - `scope`: one scope's task.
//! - `effects`: performing a scope's effects.

mod effects;
mod scope;

use std::collections::BTreeMap;
use std::sync::mpsc as std_mpsc;

use ramflux_desktop_core::{ContextId, DomainEvent, Effect, MomentId, Timestamp};
use serde_json::json;
use tokio::sync::mpsc;

use crate::bus::client::BusClient;
use crate::bus::{BusError, BusRequest};
use crate::config::SdkConfig;
use crate::translate::Scope;

/// One fact for a source, stamped when the bridge observed it.
#[derive(Debug)]
pub struct Delivery {
    /// Observation time.
    pub at: Timestamp,
    /// The fact.
    pub event: DomainEvent,
}

/// A request from a source to the bridge.
pub(crate) enum Command {
    /// Start serving `scope`, delivering into `outbox`.
    Attach { scope: Scope, outbox: std_mpsc::Sender<Delivery> },
    /// Perform an effect in `scope`.
    Effect { scope: Scope, effect: Effect },
    /// The user looks at `context` in `scope`.
    Focus { scope: Scope, context: Option<ContextId> },
    /// A view needs these messages' bodies again in `scope`.
    Bodies { scope: Scope, moments: Vec<MomentId> },
    /// Stop serving `scope`.
    Detach { scope: Scope },
    /// Ask the daemon to stop (`daemon.stop`) and report its answer.
    StopDaemon { reply: std_mpsc::Sender<Result<(), BusError>> },
}

/// Handle to the bridge thread. Dropping every handle stops it.
#[derive(Clone)]
pub struct Bridge {
    commands: mpsc::UnboundedSender<Command>,
}

impl Bridge {
    /// Starts the bridge thread.
    ///
    /// # Errors
    ///
    /// Returns the I/O error when the thread or its runtime cannot start.
    pub fn start(config: SdkConfig) -> std::io::Result<Self> {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
        let (commands, inbox) = mpsc::unbounded_channel();
        std::thread::Builder::new()
            .name("ramflux-sdk-bridge".to_owned())
            .spawn(move || runtime.block_on(serve(config, inbox)))?;
        Ok(Self { commands })
    }

    /// Queues a command; a stopped bridge drops it.
    pub(crate) fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }
}

async fn serve(config: SdkConfig, mut inbox: mpsc::UnboundedReceiver<Command>) {
    let mut scopes: BTreeMap<Scope, mpsc::UnboundedSender<scope::ScopeCommand>> = BTreeMap::new();
    while let Some(command) = inbox.recv().await {
        match command {
            Command::Attach { scope, outbox } => {
                let (tx, rx) = mpsc::unbounded_channel();
                tokio::spawn(scope::run(config.clone(), scope.clone(), outbox, rx));
                scopes.insert(scope, tx);
            }
            Command::Effect { scope, effect } => {
                if let Some(tx) = scopes.get(&scope) {
                    let _ = tx.send(scope::ScopeCommand::Effect(Box::new(effect)));
                }
            }
            Command::Focus { scope, context } => {
                if let Some(tx) = scopes.get(&scope) {
                    let _ = tx.send(scope::ScopeCommand::Focus(context));
                }
            }
            Command::Bodies { scope, moments } => {
                if let Some(tx) = scopes.get(&scope) {
                    let _ = tx.send(scope::ScopeCommand::Bodies(moments));
                }
            }
            Command::Detach { scope } => {
                scopes.remove(&scope);
            }
            Command::StopDaemon { reply } => {
                let client = BusClient::new(
                    config.socket_path.clone(),
                    config.connect_timeout,
                    config.deadlines,
                );
                tokio::spawn(async move {
                    let request = BusRequest::new(None, "daemon", "daemon.stop", json!({}));
                    let answer = client.call(&request).await.and_then(stopping);
                    let _ = reply.send(answer);
                });
            }
        }
    }
}

/// Checks the answer to `daemon.stop`.
fn stopping(answer: serde_json::Value) -> Result<(), BusError> {
    if answer.get("stopping").and_then(serde_json::Value::as_bool) == Some(true) {
        Ok(())
    } else {
        Err(BusError::Transport(format!("unexpected answer to daemon.stop: {answer}")))
    }
}

/// Wall-clock time as a contract timestamp.
pub(crate) fn now() -> Timestamp {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis());
    Timestamp(i64::try_from(millis).unwrap_or(i64::MAX))
}
