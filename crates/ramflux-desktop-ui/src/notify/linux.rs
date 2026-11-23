// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The Linux sender: `org.freedesktop.Notifications` over the session bus,
//! through `notify-rust` (the crate GPUI sends its own notifications with).
//!
//! Each notification is shown, and then waited on, on a short-lived thread
//! of its own (showing connects to the session bus; waiting blocks until the
//! notification is clicked or closed), as GPUI does. A notification for the
//! same conversation and kind replaces the live one through its server id.
//! When a replaced notification is clicked, the server reports the click
//! once for that id and every waiting thread of the id sees it; only the
//! newest one reports it to the window.
//!
//! No session bus or no notification server means no notification and one
//! warning in the log per failed show; the app goes on.

use std::collections::HashMap;
use std::sync::Arc;

use notify_rust::{Hint, Notification, Urgency as XdgUrgency};
use parking_lot::Mutex;
use ramflux_desktop_sdk::local::app_log;
use ramflux_desktop_sdk::local::identity::{APP_ID, APP_NAME};

use super::{Activation, ActivationSink, Notifier, Shown, Urgency, escape_markup};

/// The action key servers invoke when the notification itself is clicked.
const DEFAULT_ACTION: &str = "default";

#[derive(Default)]
struct State {
    /// Server id and generation of the live notification per replace key.
    live: HashMap<String, (u32, u64)>,
    generation: u64,
    /// Whether the server interprets body markup (asked once).
    markup: Option<bool>,
}

/// Sends through the freedesktop notifications interface.
pub struct LinuxNotifier {
    sink: ActivationSink,
    state: Arc<Mutex<State>>,
}

impl LinuxNotifier {
    /// A sender reporting clicks to `sink`.
    pub fn new(sink: ActivationSink) -> Self {
        Self { sink, state: Arc::default() }
    }
}

impl Notifier for LinuxNotifier {
    fn show(&self, shown: &Shown) {
        let shown = shown.clone();
        let sink = self.sink.clone();
        let state = self.state.clone();
        let spawned = std::thread::Builder::new()
            .name("notification".to_owned())
            .spawn(move || deliver(&shown, &sink, &state));
        if let Err(error) = spawned {
            app_log::warn(&format!("cannot start a notification thread: {error}"));
        }
    }
}

fn body_markup(state: &Mutex<State>) -> bool {
    if let Some(known) = state.lock().markup {
        return known;
    }
    let markup = notify_rust::get_capabilities()
        .map(|caps| caps.iter().any(|c| c == "body-markup"))
        // Unknown: escaping shows entities as text on a plain server, which
        // is safer than letting a markup server interpret message text.
        .unwrap_or(true);
    state.lock().markup = Some(markup);
    markup
}

fn deliver(shown: &Shown, sink: &ActivationSink, state: &Mutex<State>) {
    let body = if body_markup(state) { escape_markup(&shown.body) } else { shown.body.clone() };
    let (replaces, generation) = {
        let mut state = state.lock();
        state.generation += 1;
        let replaces =
            shown.replace_key.as_ref().and_then(|key| state.live.get(key)).map(|(id, _)| *id);
        (replaces, state.generation)
    };
    let mut notification = Notification::new();
    notification
        .appname(APP_NAME)
        .summary(&shown.summary)
        .body(&body)
        .icon(APP_ID)
        .hint(Hint::DesktopEntry(APP_ID.to_owned()))
        .hint(Hint::Transient(shown.transient))
        .urgency(match shown.urgency {
            Urgency::Normal => XdgUrgency::Normal,
            Urgency::Critical => XdgUrgency::Critical,
        })
        .action(DEFAULT_ACTION, &shown.open_label);
    if let Some(id) = replaces {
        notification.id(id);
    }
    let handle = match notification.show() {
        Ok(handle) => handle,
        Err(error) => {
            app_log::warn(&format!("cannot show a notification: {error}"));
            return;
        }
    };
    if let Some(key) = &shown.replace_key {
        let mut state = state.lock();
        // A newer notice for the key may have been shown first (threads
        // race); it stays the one clicks are reported for.
        if state.live.get(key).is_none_or(|(_, newest)| *newest < generation) {
            state.live.insert(key.clone(), (handle.id(), generation));
        }
    }
    let is_newest = |state: &State| {
        shown
            .replace_key
            .as_ref()
            .is_none_or(|key| state.live.get(key).is_some_and(|(_, newest)| *newest == generation))
    };
    handle.wait_for_action(|action| {
        if action == DEFAULT_ACTION && is_newest(&state.lock()) {
            sink.activate(Activation { conversation: shown.conversation.clone() });
        }
    });
    // Clicked or closed: the next notice for this key starts a new one.
    let mut state = state.lock();
    if is_newest(&state)
        && let Some(key) = &shown.replace_key
    {
        state.live.remove(key);
    }
}
