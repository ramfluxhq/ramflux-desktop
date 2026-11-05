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
