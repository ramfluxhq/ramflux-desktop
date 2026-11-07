// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Deterministic completion ordering for integration regressions.
use std::sync::Mutex;

/// Holds one secret handoff until the test has observed its ready event.
#[derive(Default)]
pub struct PhraseHandoffGate {
    release: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}
impl std::fmt::Debug for PhraseHandoffGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PhraseHandoffGate").finish_non_exhaustive()
    }
}
impl PhraseHandoffGate {
    pub(crate) fn hold(&self, release: impl FnOnce() + Send + 'static) {
        *self.release.lock().expect("handoff gate") = Some(Box::new(release));
    }
    /// Whether the bridge has parked the phrase, without exposing its words.
    pub fn waiting(&self) -> bool {
        self.release.lock().expect("handoff gate").is_some()
    }
    /// Publishes the held handoff through its original completion sender.
    pub fn release(&self) {
        let release = self.release.lock().expect("handoff gate").take();
        if let Some(release) = release {
            release();
        }
    }
}

type Registry = Mutex<
    Vec<(std::path::PathBuf, tokio::sync::mpsc::WeakUnboundedSender<crate::bridge::Command>)>,
>;

/// Bridges by daemon socket, held weakly so a dropped hub still stops its
/// bridge.
static BRIDGES: Registry = Mutex::new(Vec::new());

pub(crate) fn register_bridge(
    socket: &std::path::Path,
    commands: &tokio::sync::mpsc::UnboundedSender<crate::bridge::Command>,
) {
    let mut bridges = BRIDGES.lock().expect("bridge registry");
    bridges.retain(|(path, sender)| path != socket && sender.upgrade().is_some());
    bridges.push((socket.to_path_buf(), commands.downgrade()));
}

/// Routes a generated message update into `account`'s scope of the bridge
/// serving `socket`, through `translate::v1::candidate::apply`. Returns
/// whether such a bridge is running.
pub fn push_generated(
    socket: &std::path::Path,
    account: &ramflux_desktop_core::AccountId,
    update: ramflux_protocol::message::MessageUpdate,
) -> bool {
    let sender = BRIDGES
        .lock()
        .expect("bridge registry")
        .iter()
        .find(|(path, _)| path == socket)
        .and_then(|(_, sender)| sender.upgrade());
    sender.is_some_and(|sender| {
        sender
            .send(crate::bridge::Command::Generated {
                scope: crate::translate::Scope::Account(account.clone()),
                updates: vec![update],
            })
            .is_ok()
    })
}
