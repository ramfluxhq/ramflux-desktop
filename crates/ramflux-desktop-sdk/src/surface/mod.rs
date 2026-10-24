// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Agent surfaces on the daemon: what the desktop reads and sends.
//!
//! The daemon applies every surface event (only through the protocol's
//! validation of the original signed bytes), keeps the current state per
//! surface scoped by conversation, author and surface id (`surface_ref`),
//! and computes the protocol's snapshot hash. The desktop never hashes or
//! re-validates a surface: it shows the daemon's state and binds actions to
//! the version and hash the daemon reported.
//!
//! - [`catalog`]: snapshots as desktop blocks and actions, and back.
//! - [`action`]: a surface action as the protocol's action bytes.
//! - [`author`]: desktop surface events as an author's protocol events
//!   (fixtures and replays).

pub mod action;
pub mod author;
pub mod catalog;

/// Where a surface lives, as the daemon scopes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceTarget {
    /// The conversation.
    pub conversation_id: String,
    /// The author's principal.
    pub author_principal_id: String,
    /// The author's surface id (unique per author and conversation only).
    pub surface_id: String,
}

#[cfg(test)]
mod tests {
    use ramflux_desktop_core::conformance::Limits;

    #[test]
    fn the_renderer_limits_are_the_protocols() {
        let limits = Limits::DESKTOP;
        assert_eq!(limits.max_blocks, ramflux_protocol::MAX_SURFACE_BLOCKS);
        assert_eq!(limits.max_actions, ramflux_protocol::MAX_SURFACE_ACTIONS);
        assert_eq!(limits.max_text_chars, ramflux_protocol::MAX_SURFACE_TEXT_CHARS);
        assert_eq!(limits.max_data_bytes, ramflux_protocol::MAX_SURFACE_DATA_BYTES);
        assert_eq!(limits.max_text_delta_bytes, ramflux_protocol::MAX_SURFACE_TEXT_DELTA_BYTES);
        // Streamed text is bounded by the same state bound.
        assert_eq!(limits.max_stream_chars, ramflux_protocol::MAX_SURFACE_DATA_BYTES);
    }

    /// The largest state the protocol accepts (16,384 canonical bytes) is
    /// within the desktop's limits: the desktop never shows only the
    /// fallback for a surface the daemon holds. (The desktop counts block
    /// data only, which is less than the protocol's whole snapshot.)
    #[test]
    fn the_largest_protocol_state_renders_on_the_desktop() {
        use ramflux_desktop_core::conformance::over_limit;
        use ramflux_desktop_core::{Block, BlockKind, ContextId, MomentId, ParticipantId};
        use ramflux_desktop_core::{SurfaceEvent, SurfaceId};
        use ramflux_protocol::{A2uiConversationEvent, SurfaceState};
        use serde_json::json;

        for unit in ["x", "\u{e9}", "\u{1f600}"] {
            let accepted = |n: usize| {
                let create = SurfaceEvent::Create {
                    moment_id: MomentId::new("m"),
                    context_id: ContextId::new("c"),
                    origin: ParticipantId::new("p"),
                    surface_id: SurfaceId::new("s"),
                    version: 1,
                    blocks: vec![Block::new(
                        "b",
                        BlockKind::Text,
                        json!({ "text": unit.repeat(n) }),
                    )],
                    actions: Vec::new(),
                    fallback_text: "Reply".into(),
                    hash: None,
                };
                let mut state = None;
                let mut author = super::author::Author::default();
                let sent = author.send("c", &create, |bytes| {
                    let event = A2uiConversationEvent::from_wire_bytes(bytes).expect("event");
                    state = SurfaceState::default().apply(&event).ok();
                    state.is_some()
                });
                sent.ok().filter(|ok| *ok).and(state)
            };
            let (mut low, mut high) = (0, ramflux_protocol::MAX_SURFACE_DATA_BYTES + 1);
            while high - low > 1 {
                let mid = usize::midpoint(low, high);
                if accepted(mid).is_some() { low = mid } else { high = mid }
            }
            assert!(low * unit.len() > 16_000, "near the cap: {unit} x {low}");
            let state = accepted(low).expect("the largest accepted state");
            let snapshot = serde_json::to_value(state.content()).expect("snapshot");
            let payload = super::catalog::payload_of(&snapshot, SurfaceId::new("s"));
            assert_eq!(over_limit(&payload, &Limits::DESKTOP), None, "{unit} x {low}");
        }
    }
}
