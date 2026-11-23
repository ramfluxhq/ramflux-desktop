// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Operating-system notifications with preview privacy.
//!
//! A [`Notice`] says what happened (a message, an approval request, an
//! incoming call), in which conversation and from whom. [`compose`] decides,
//! from the device's [`PreviewLevel`], what the notification shows; a
//! [`Notifier`] hands it to the platform. The decision is platform-neutral
//! and unit-tested; only delivery is per platform:
//!
//! - **Linux** ([`linux`]): the freedesktop notifications D-Bus interface,
//!   with the `desktop-entry` hint (the app id, so the server finds the
//!   launcher entry and icon), the `transient` hint on every notification
//!   that carries a name or text (servers such as Omarchy's keep a history
//!   on disk; transient notifications are never written to it), urgency
//!   (critical for calls), one replaceable notification per conversation and
//!   kind, and a `default` action: clicking the notification brings the
//!   window forward and opens the conversation.
//! - **Elsewhere**: a no-op for now (macOS may get its own sender later).
//!
//! GPUI Kit's own notification API (`gpui-pre-linux`,
//! `system_notifications.rs`) sets only the application name and actions,
//! so it cannot carry these hints; the Linux sender uses `notify-rust`, the
//! crate GPUI itself sends through (already in the dependency graph).
//!
//! What raises a notice (focus, mute, which events) is wired in a later
//! step; this module is the sending half and the click-through.

#[cfg(target_os = "linux")]
pub mod linux;

use std::sync::mpsc;

use gpui_kit::{AnyWindowHandle, App, AppContext as _, Entity, Global};
use ramflux_desktop_core::ContextId;
use ramflux_desktop_core::source::SourceWake;
use ramflux_desktop_sdk::local::{NotificationSettings, NotificationStore, PreviewLevel, app_log};
use unicode_segmentation::UnicodeSegmentation as _;

use crate::i18n::{tr, tr_args};
use crate::shell::Shell;

/// The longest text preview shown, in grapheme clusters.
pub const PREVIEW_GRAPHEMES: usize = 160;

/// What a notice is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeKind {
    /// A message arrived.
    Message,
    /// Someone (usually an agent) asks for an approval.
    Approval,
    /// An incoming call rings.
    Call,
}

impl NoticeKind {
    fn tag(self) -> &'static str {
        match self {
            Self::Message => "message",
            Self::Approval => "approval",
            Self::Call => "call",
        }
    }
}

/// Something worth telling the user about outside the window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    /// What happened.
    pub kind: NoticeKind,
    /// The conversation it belongs to (opened on click).
    pub conversation: Option<ContextId>,
    /// Who sent it (a display name).
    pub sender: Option<String>,
    /// The message text, or what the approval asks for.
    pub text: Option<String>,
}

/// How insistently the notification server should present it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Urgency {
    /// Ordinary.
    Normal,
    /// Needs attention now (stays until handled on most servers).
    Critical,
}

/// What a notification shows and how it behaves, decided by [`compose`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    /// The first line.
    pub summary: String,
    /// The rest (may be empty).
    pub body: String,
    /// Whether the server must not keep it in a history: set whenever the
    /// summary or body carries anything taken from the notice (a name or
    /// text).
    pub transient: bool,
    /// Urgency.
    pub urgency: Urgency,
    /// One live notification per key: a new one with the same key replaces
    /// the old one (per conversation and kind).
    pub replace_key: Option<String>,
    /// The conversation the click opens.
    pub conversation: Option<ContextId>,
    /// The label of the click action (some servers show it).
    pub open_label: String,
}

fn nonempty(value: Option<&String>) -> Option<&str> {
    value.map(|v| v.trim()).filter(|v| !v.is_empty())
}

/// A text preview: control characters other than line breaks removed,
/// shortened to [`PREVIEW_GRAPHEMES`].
pub fn preview(text: &str) -> String {
    let clean: String = text.chars().filter(|c| *c == '\n' || !c.is_control()).collect::<String>();
    let clean = clean.trim();
    let mut graphemes = clean.graphemes(true);
    let mut out: String = graphemes.by_ref().take(PREVIEW_GRAPHEMES).collect();
    if graphemes.next().is_some() {
        out.push('\u{2026}');
    }
    out
}

/// What `notice` shows at `level` (see [`PreviewLevel`]).
///
/// - [`PreviewLevel::SenderAndText`]: the sender and the text.
/// - [`PreviewLevel::SenderOnly`]: the sender and a generic line.
/// - [`PreviewLevel::Nothing`]: a generic line only.
///
/// [`Shown::transient`] is set whenever the result holds the sender or the
/// text.
pub fn compose(notice: &Notice, level: PreviewLevel) -> Shown {
    let sender = match level {
        PreviewLevel::Nothing => None,
        _ => nonempty(notice.sender.as_ref()),
    };
    let text = match level {
        PreviewLevel::SenderAndText => nonempty(notice.text.as_ref()).map(preview),
        _ => None,
    };
    let title = match notice.kind {
        NoticeKind::Message => tr("notify.message.title"),
        NoticeKind::Approval => tr("notify.approval.title"),
        NoticeKind::Call => tr("notify.call.title"),
    }
    .to_string();
    let summary = match (notice.kind, sender) {
        (NoticeKind::Message, Some(sender)) => sender.to_owned(),
        (NoticeKind::Approval, Some(sender)) => {
            tr_args("notify.approval.from", &[("sender", sender)]).to_string()
        }
        (NoticeKind::Call, Some(sender)) => {
            tr_args("notify.call.from", &[("sender", sender)]).to_string()
        }
        (_, None) => title.clone(),
    };
    let body = match (&text, notice.kind, sender) {
        (Some(text), _, _) => text.clone(),
        // The sender is the summary: say what it is about underneath.
        (None, NoticeKind::Message, Some(_)) => title,
        (None, _, _) => String::new(),
    };
    Shown {
        summary,
        body,
        transient: sender.is_some() || text.is_some(),
        urgency: match notice.kind {
            NoticeKind::Call => Urgency::Critical,
            NoticeKind::Message | NoticeKind::Approval => Urgency::Normal,
        },
        replace_key: notice
            .conversation
            .as_ref()
            .map(|id| format!("{}:{}", notice.kind.tag(), id.as_str())),
        conversation: notice.conversation.clone(),
        open_label: tr("notify.action.open").to_string(),
    }
}

/// Escapes text for a server that interprets body markup.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn escape_markup(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// A click on a notification: bring the window forward and open this.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Activation {
    /// The conversation to open, if any.
    pub conversation: Option<ContextId>,
}

/// Where a platform sender reports clicks (from any thread).
#[derive(Clone)]
pub struct ActivationSink {
    sender: mpsc::Sender<Activation>,
    wake: SourceWake,
}

impl ActivationSink {
    /// A sink and the receiving end the window drains when woken.
    pub fn channel() -> (Self, mpsc::Receiver<Activation>, SourceWake) {
        let (sender, receiver) = mpsc::channel();
        let wake = SourceWake::default();
        (Self { sender, wake: wake.clone() }, receiver, wake)
    }

    /// Reports a click.
    pub fn activate(&self, activation: Activation) {
        if self.sender.send(activation).is_ok() {
            self.wake.notify();
        }
    }
}

/// Hands composed notifications to the platform.
pub trait Notifier {
    /// Shows `shown`, replacing the live one with the same key. Never
    /// blocks the caller; failures are logged, not returned.
    fn show(&self, shown: &Shown);
}

/// Shows nothing (platforms without a sender yet, tests).
#[derive(Debug, Default)]
pub struct NoopNotifier;

impl Notifier for NoopNotifier {
    fn show(&self, _: &Shown) {}
}

/// The sender of this platform.
pub fn platform_notifier(sink: ActivationSink) -> Box<dyn Notifier> {
    #[cfg(target_os = "linux")]
    {
        Box::new(linux::LinuxNotifier::new(sink))
    }
    #[cfg(not(target_os = "linux"))]
    {
        drop(sink);
        Box::new(NoopNotifier)
    }
}

/// The application's notification service.
pub struct Notifications {
    notifier: Box<dyn Notifier>,
    store: NotificationStore,
    settings: NotificationSettings,
}

impl Global for Notifications {}

impl Notifications {
    /// A service sending through `notifier` with choices kept in `store`.
    pub fn new(notifier: Box<dyn Notifier>, store: NotificationStore) -> Self {
        let settings = store.load();
        Self { notifier, store, settings }
    }

    /// The preview level in force.
    pub fn preview(&self) -> PreviewLevel {
        self.settings.preview_or_default()
    }
}

/// Installs the service for the application window: the platform sender,
/// the saved preview level, and the click-through to `window`.
pub fn install(window: AnyWindowHandle, shell: Entity<Shell>, cx: &mut App) {
    let (sink, clicks, wake) = ActivationSink::channel();
    cx.set_global(Notifications::new(
        platform_notifier(sink),
        NotificationStore::default_location(),
    ));
    cx.spawn(async move |cx| {
        loop {
            wake.notified().await;
            while let Ok(activation) = clicks.try_recv() {
                let opened = cx.update_window(window, |_, window, cx| {
                    shell.update(cx, |shell, cx| open(shell, &activation, window, cx));
                });
                if opened.is_err() {
                    return;
                }
            }
        }
    })
    .detach();
}

/// Brings the window forward and shows the conversation of `activation`.
fn open(
    shell: &mut Shell,
    activation: &Activation,
    window: &mut gpui_kit::Window,
    cx: &mut gpui_kit::Context<Shell>,
) {
    window.activate_window();
    let Some(id) = activation.conversation.clone() else { return };
    if shell.is_settings_open() {
        shell.close_settings_now(cx);
    }
    shell.workspace().update(cx, |workspace, cx| workspace.select_context(Some(id), window, cx));
}

/// Shows `notice` as the device's preview level allows. Does nothing before
/// [`install`].
pub fn post(notice: &Notice, cx: &App) {
    if let Some(service) = cx.try_global::<Notifications>() {
        service.notifier.show(&compose(notice, service.preview()));
    }
}

/// Changes and saves the preview level.
pub fn set_preview(level: PreviewLevel, cx: &mut App) {
    if !cx.has_global::<Notifications>() {
        return;
    }
    let service = cx.global_mut::<Notifications>();
    service.settings.preview = Some(level);
    if let Err(error) = service.store.save(&service.settings) {
        app_log::warn(&format!("cannot save the notification choice: {error}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SENDER: &str = "Mira Okafor";
    const TEXT: &str = "the vault code is 4417";

    fn notice(kind: NoticeKind) -> Notice {
        Notice {
            kind,
            conversation: Some(ContextId::new("ctx-1")),
            sender: Some(SENDER.into()),
            text: Some(TEXT.into()),
        }
    }

    fn shows(shown: &Shown, needle: &str) -> bool {
        shown.summary.contains(needle) || shown.body.contains(needle)
    }

    #[test]
    fn each_level_reveals_only_what_it_allows() {
        for kind in [NoticeKind::Message, NoticeKind::Approval, NoticeKind::Call] {
            let full = compose(&notice(kind), PreviewLevel::SenderAndText);
            assert!(shows(&full, SENDER) && shows(&full, TEXT), "{kind:?}: {full:?}");

            let sender = compose(&notice(kind), PreviewLevel::SenderOnly);
            assert!(shows(&sender, SENDER), "{kind:?}: {sender:?}");
            assert!(!shows(&sender, TEXT), "{kind:?}: no text at sender only: {sender:?}");

            let nothing = compose(&notice(kind), PreviewLevel::Nothing);
            assert!(!shows(&nothing, SENDER) && !shows(&nothing, TEXT), "{kind:?}: {nothing:?}");
            assert!(!nothing.summary.is_empty(), "a generic line is still shown");
        }
    }

    #[test]
    fn transient_whenever_content_is_shown() {
        let variants = [
            notice(NoticeKind::Message),
            Notice { text: None, ..notice(NoticeKind::Message) },
            Notice { sender: None, ..notice(NoticeKind::Message) },
            Notice {
                sender: Some("  ".into()),
                text: Some(String::new()),
                ..notice(NoticeKind::Call)
            },
            notice(NoticeKind::Approval),
            notice(NoticeKind::Call),
        ];
        for n in &variants {
            for level in PreviewLevel::ALL {
                let shown = compose(n, level);
                let content = [n.sender.as_deref(), n.text.as_deref()]
                    .into_iter()
                    .flatten()
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .any(|v| shows(&shown, v));
                assert_eq!(shown.transient, content, "{level:?} {n:?}: {shown:?}");
                if content {
                    assert!(shown.transient, "content is never persisted");
                }
            }
        }
        // A generic notice may be kept in a history.
        assert!(!compose(&notice(NoticeKind::Message), PreviewLevel::Nothing).transient);
    }

    #[test]
    fn the_default_level_never_shows_text() {
        let shown = compose(&notice(NoticeKind::Message), PreviewLevel::default());
        assert!(!shows(&shown, TEXT));
        assert!(shown.transient);
    }

    #[test]
    fn urgency_replacement_and_click_target() {
        let call = compose(&notice(NoticeKind::Call), PreviewLevel::SenderOnly);
        assert_eq!(call.urgency, Urgency::Critical);
        let message = compose(&notice(NoticeKind::Message), PreviewLevel::SenderOnly);
        assert_eq!(message.urgency, Urgency::Normal);
        assert_eq!(message.replace_key.as_deref(), Some("message:ctx-1"));
        assert_eq!(
            call.replace_key.as_deref(),
            Some("call:ctx-1"),
            "kinds do not replace each other"
        );
        assert_eq!(message.conversation, Some(ContextId::new("ctx-1")));
        assert!(!message.open_label.is_empty());
        let loose = Notice { conversation: None, ..notice(NoticeKind::Approval) };
        assert_eq!(compose(&loose, PreviewLevel::Nothing).replace_key, None);
    }

    #[test]
    fn previews_are_short_and_clean() {
        let long = "word ".repeat(100);
        let shown = preview(&long);
        assert_eq!(shown.graphemes(true).count(), PREVIEW_GRAPHEMES + 1);
        assert!(shown.ends_with('\u{2026}'));
        assert_eq!(preview("a\u{7}b\nc\u{1b}[31m"), "ab\nc[31m");
        assert_eq!(preview("  short  "), "short");
    }

    #[test]
    fn markup_is_escaped() {
        assert_eq!(escape_markup("<b>a & 'b'</b>"), "&lt;b&gt;a &amp; &#39;b&#39;&lt;/b&gt;");
    }

    #[test]
    fn clicks_reach_the_receiver_and_wake_it() {
        let (sink, clicks, _wake) = ActivationSink::channel();
        let click = Activation { conversation: Some(ContextId::new("ctx-9")) };
        std::thread::spawn({
            let sink = sink.clone();
            let click = click.clone();
            move || sink.activate(click)
        })
        .join()
        .expect("thread");
        assert_eq!(clicks.try_recv().ok(), Some(click));
    }
}
