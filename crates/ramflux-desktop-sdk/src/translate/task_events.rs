// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Task event messages as compact timeline lines.
//!
//! Every signed task event travels as a conversation message. The daemon
//! lists it with `content_type: "task"` and its `task_id`, and as its text
//! (`body_utf8`) the protocol's text rendering of the verified event
//! (`TaskEvent::fallback_text`):
//!
//! ```text
//! Signed event signer="device:<id>" kind="<kind>" content="<lines>"
//! ```
//!
//! followed by ` (actor kind unverified)` when the actor's kind is not
//! verified and the rendering does not already say so. The row also
//! carries `actor_kind_verified`: whether a root-signed profile and a
//! current device of the signer prove its kind for the event's role. The
//! line shows that flag; a row without it (a sibling copy) is unverified:
//! a trust mark is never inferred from a missing string.
//!
//! The task card shows the task itself; what a member did to it besides
//! the assignee's updates becomes a [`MomentKind::TaskEvent`] line by the
//! member who sent it (the row's sender):
//!
//! | `kind` | line |
//! |---|---|
//! | `accept` | `Accepted` |
//! | `decline` | `Declined` |
//! | `cancel` | `Cancelled` |
//! | `resolve` (`decision … resume for step <id> at …`) | `Approved { step }` |
//! | `resolve` (`decision … deny for step <id> at …`) | `Denied { step }` |
//! | `start`, `proposal`, `update` | none (the card shows them) |
//!
//! The text is never shown. A sibling device's copy of this account's own
//! event can reach the message list without the task fields (seen on a
//! second device of the same account); a row from this account whose text
//! is exactly that rendering of a task event is read as the task event it
//! names (its task is read by the id the text names), never as a message.
//! A peer's row is only ever read by its task fields.

use ramflux_desktop_core::{
    ContextId, Lifecycle, Moment, MomentKind, StepId, TaskEventKind, TaskEventPayload, Timestamp,
};
use serde_json::Value;

use super::conversation::message_moment_id;
use super::tasks::task_key;
use super::{Translator, seconds, text};

/// The fields of a task event's text rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Rendered {
    /// The signing key id (`device:<id>`).
    pub signer: String,
    /// The event kind (`accept`, `resolve`, ...).
    pub kind: String,
    /// The event's lines.
    pub content: String,
}

/// What the daemon appends to a rendering that does not say the actor's
/// kind is unverified.
const UNVERIFIED: &str = " (actor kind unverified)";

/// The event kinds of a task (`TaskEventBody::kind`).
const TASK_KINDS: [&str; 7] =
    ["start", "proposal", "accept", "decline", "update", "cancel", "resolve"];

/// Parses `Signed event signer="…" kind="…" content="…"`, undoing the
/// protocol's quoting (`\"`, `\\`, `\n`, `\r`, `\t`, `\u{…}`). Anything
/// else (a prefix, a missing field, trailing text) is not a rendering.
pub(crate) fn parse(text: &str) -> Option<Rendered> {
    let rest = text.strip_prefix("Signed event signer=")?;
    let (signer, rest) = quoted(rest)?;
    let rest = rest.strip_prefix(" kind=")?;
    let (kind, rest) = quoted(rest)?;
    let rest = rest.strip_prefix(" content=")?;
    let (content, rest) = quoted(rest)?;
    // The daemon names an unverified actor kind after a rendering that does
    // not name it itself.
    let rest = rest.strip_prefix(UNVERIFIED).unwrap_or(rest);
    rest.is_empty().then_some(Rendered { signer, kind, content })
}

/// One quoted field at the start of `text`, and what follows it.
fn quoted(text: &str) -> Option<(String, &str)> {
    let mut chars = text.strip_prefix('"')?.char_indices();
    let mut out = String::new();
    while let Some((ix, ch)) = chars.next() {
        match ch {
            '"' => return Some((out, &text[ix + 2..])),
            '\\' => match chars.next()?.1 {
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                'u' => {
                    if chars.next()?.1 != '{' {
                        return None;
                    }
                    let mut hex = String::new();
                    loop {
                        match chars.next()?.1 {
                            '}' => break,
                            digit => hex.push(digit),
                        }
                    }
                    out.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
                }
                other => out.push(other),
            },
            ch => out.push(ch),
        }
    }
    None
}

impl Rendered {
    /// Whether it renders a task event.
    fn is_task(&self) -> bool {
        TASK_KINDS.contains(&self.kind.as_str())
    }

    /// The task id its content names (`Task <id> …` / `Task <id>: …`).
    fn task_id(&self) -> Option<&str> {
        let rest = self.content.strip_prefix("Task ")?;
        let end = rest.find([' ', ':', '\n']).unwrap_or(rest.len());
        Some(&rest[..end]).filter(|id| !id.is_empty())
    }

    /// The line it becomes, if any.
    fn event(&self) -> Option<TaskEventKind> {
        match self.kind.as_str() {
            "accept" => Some(TaskEventKind::Accepted),
            "decline" => Some(TaskEventKind::Declined),
            "cancel" => Some(TaskEventKind::Cancelled),
            "resolve" => {
                let line = self.content.lines().next().unwrap_or_default();
                let step = |word: &str| {
                    let rest = line.split_once(&format!(" {word} for step "))?.1;
                    let id = rest.split_once(" at ").map_or(rest, |(id, _)| id);
                    Some(StepId::new(id)).filter(|id| !id.as_str().is_empty())
                };
                if line.contains(" resume for step ") {
                    Some(TaskEventKind::Approved { step_id: step("resume") })
                } else if line.contains(" deny for step ") {
                    Some(TaskEventKind::Denied { step_id: step("deny") })
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

/// The text of a message row (`body_utf8`, else the stored body bytes).
pub(super) fn row_text(record: &Value) -> Option<String> {
    record["body_utf8"].as_str().map(str::to_owned).or_else(|| {
        let bytes: Option<Vec<u8>> = record["encrypted_body"]
            .as_array()?
            .iter()
            .map(|b| b.as_u64().and_then(|v| u8::try_from(v).ok()))
            .collect();
        bytes.and_then(|b| String::from_utf8(b).ok())
    })
}

impl Translator {
    /// The line of a task message row (`content_type: "task"`), if its
    /// event is one.
    pub(super) fn task_event_moment(
        &self,
        context: &ContextId,
        record: &Value,
        task_id: &str,
        now: Timestamp,
    ) -> Option<Moment> {
        let message = text(record, "message_id");
        if message.is_empty() || task_id.is_empty() {
            return None;
        }
        let event = parse(&row_text(record)?)?.event()?;
        // Absent (a sibling copy): unverified, never inferred from text.
        let actor_kind_verified = record["actor_kind_verified"].as_bool().unwrap_or(false);
        // The authenticated kind (null for unknown or legacy metadata): never
        // inferred from the role or the flag.
        let actor_kind = record["actor_kind"].as_str().and_then(super::contacts::peer_kind);
        Some(Moment {
            id: message_moment_id(context, message),
            context_id: context.clone(),
            origin: self.origin(text(record, "sender_id")),
            at: seconds(&record["created_at"]).unwrap_or(now),
            kind: MomentKind::TaskEvent(TaskEventPayload {
                task_id: task_key(context.as_str(), task_id),
                event,
                actor_kind_verified,
                actor_kind,
            }),
            lifecycle: Lifecycle::Done,
        })
    }

    /// The task a row without task fields stands for: a copy of this
    /// account's own task event (sent by this account, its text exactly
    /// the event's rendering).
    pub(super) fn own_task_copy(&self, record: &Value) -> Option<String> {
        let sender = text(record, "sender_id");
        let me = self.directory.self_principal.as_deref()?;
        if sender.is_empty() || self.origin(sender).as_str() != me {
            return None;
        }
        let rendered = parse(&row_text(record)?)?;
        if !rendered.is_task() {
            return None;
        }
        rendered.task_id().map(str::to_owned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_protocol_quoting_is_undone() {
        let text = r#"Signed event signer="device:d-1" kind="resolve" content="Task t-1: decision (actor kind unverified) deny for step deploy at grant \"g\" (decision a)\nnext""#;
        let rendered = parse(text).expect("rendering");
        assert_eq!(rendered.signer, "device:d-1");
        assert_eq!(rendered.task_id(), Some("t-1"));
        assert_eq!(
            rendered.event(),
            Some(TaskEventKind::Denied { step_id: Some(StepId::new("deploy")) })
        );
        assert!(rendered.content.ends_with("(decision a)\nnext"));
        assert_eq!(parse("Signed event signer=\"x\" kind=\"accept\""), None);
        assert_eq!(parse(&format!("{text} trailing")), None);
        let cancel = r#"Signed event signer="s" kind="cancel" content="Task t-2 cancelled by its requester" (actor kind unverified)"#;
        assert_eq!(parse(cancel).and_then(|r| r.event()), Some(TaskEventKind::Cancelled));
        assert_eq!(parse("hello"), None);
        let escaped = parse(r#"Signed event signer="s" kind="accept" content="Task t \u{202e}x""#)
            .expect("escaped");
        assert_eq!(escaped.content, "Task t \u{202e}x");
    }

    #[test]
    fn updates_starts_and_proposals_are_no_line() {
        for kind in ["start", "proposal", "update"] {
            let text = format!(r#"Signed event signer="s" kind="{kind}" content="Task t-1: done""#);
            let rendered = parse(&text).expect("rendering");
            assert!(rendered.is_task());
            assert_eq!(rendered.event(), None);
        }
        let surface = parse(r#"Signed event signer="s" kind="surface" content="Pick one""#)
            .expect("rendering");
        assert!(!surface.is_task());
    }
}
