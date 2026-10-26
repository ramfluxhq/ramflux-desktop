// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! A new direct conversation with a contact, before its first message.
//!
//! Opening a draft (from a contact row, the command palette or the context
//! stack's "+") chooses the conversation's id once (a fresh random id from
//! the SDK, never derived from either identity), so a retry after a failure
//! names the same conversation. The composer then submits the first message
//! as [`UiEventKind::StartConversationRequested`]; the root turns it into
//! the `StartConversation` intent. The draft follows the outcome in the
//! store: the conversation appearing ends it ([`NewConversationEvent::Started`]),
//! a failed start shows the reason in place and keeps the typed text.
//!
//! Before anything is typed the draft runs the policy's own check
//! (`check_start_conversation`) and says why a conversation cannot start
//! yet, and what to do (accept the contact, verify a changed key). A
//! contact whose verification went stale is not refused: the draft asks
//! the user to acknowledge it first (Verify again, or Continue). The draft
//! is view-local; nothing in it reaches the store.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    Context, Entity, EventEmitter, IntoElement, Render, Role, SharedString, Subscription, Window,
    div,
};
use ramflux_desktop_core::{
    ContactLink, ContextId, IntentError, ParticipantId, Scope, Store, UiEventKind,
    check_start_conversation, methods,
};

use crate::app::{StoreEvent, StoreHandle};
use crate::bubble::EventSink;
use crate::i18n::{self, tr, tr_args};

/// Scope name of the draft's sink.
pub const SCOPE: &str = "new_conversation";

/// Where the draft is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    /// Waiting for the first message.
    Composing,
    /// The first message was submitted; `since` is the store's failure
    /// count at that time.
    Sending {
        /// Failures recorded before the submission.
        since: u64,
    },
    /// The start failed (translated reason).
    Failed(SharedString),
}

/// An open draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    /// The contact.
    pub participant: ParticipantId,
    /// The id the conversation gets.
    pub context_id: ContextId,
    /// Progress.
    pub phase: Phase,
    /// The user acknowledged the contact's stale verification (Continue).
    pub acknowledged: bool,
}

/// Reported to the workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NewConversationEvent {
    /// The conversation exists now; show it.
    Started(ContextId),
}

/// The draft view and its view-local state.
pub struct NewConversation {
    store: Entity<StoreHandle>,
    draft: Option<Draft>,
    /// Where Verify again is emitted (the root's sink, scoped to the draft).
    sink: Option<EventSink>,
    /// Why no id could be chosen (no randomness), if so.
    id_error: Option<SharedString>,
    _subscription: Subscription,
}

impl EventEmitter<NewConversationEvent> for NewConversation {}

impl NewConversation {
    /// No draft, following `store`.
    pub fn new(store: Entity<StoreHandle>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.subscribe(&store, |this, _, event: &StoreEvent, cx| {
            if matches!(event, StoreEvent::Changed(_)) {
                this.observe(cx);
            }
        });
        Self { store, draft: None, sink: None, id_error: None, _subscription: subscription }
    }

    /// Opens a draft with `participant` under a fresh conversation id,
    /// emitting Verify again through `root` (the root's sink).
    pub fn open(&mut self, participant: ParticipantId, root: &EventSink, cx: &mut Context<Self>) {
        self.sink = Some(root.scoped(Scope::Panel(SCOPE.to_owned())));
        match ramflux_desktop_sdk::fresh_conversation_id() {
            Ok(context_id) => {
                self.id_error = None;
                self.draft = Some(Draft {
                    participant,
                    context_id,
                    phase: Phase::Composing,
                    acknowledged: false,
                });
            }
            Err(error) => {
                self.id_error = Some(SharedString::from(error));
                self.draft = None;
            }
        }
        cx.notify();
    }

    /// Closes the draft.
    pub fn close(&mut self, cx: &mut Context<Self>) {
        if self.draft.take().is_some() || self.id_error.take().is_some() {
            cx.notify();
        }
    }

    /// The open draft.
    pub fn draft(&self) -> Option<&Draft> {
        self.draft.as_ref()
    }

    /// Whether a draft (or its id failure) is shown.
    pub fn is_open(&self) -> bool {
        self.draft.is_some() || self.id_error.is_some()
    }

    /// Why the composer cannot submit now, if it cannot: the conversation
    /// cannot be started with this contact, or the first message is on
    /// its way.
    pub fn blocked(&self, store: &Store) -> Option<SharedString> {
        if let Some(error) = &self.id_error {
            return Some(error.clone());
        }
        let draft = self.draft.as_ref()?;
        if matches!(draft.phase, Phase::Sending { .. }) {
            return Some(tr("conversation.new.sending"));
        }
        if let Some(reason) = i18n::unsupported(store, methods::CONVERSATION_START) {
            return Some(reason);
        }
        if let Err(error) = check_start_conversation(store, &draft.participant) {
            return Some(refusal(store, &draft.participant, &error));
        }
        self.awaits_acknowledgement(store)
            .then(|| tr_args("identity.ack.wait", &[("name", &name(store, &draft.participant))]))
    }

    /// Whether the contact's verification went stale and the user has not
    /// acknowledged it yet.
    pub fn awaits_acknowledgement(&self, store: &Store) -> bool {
        self.draft.as_ref().is_some_and(|draft| {
            !draft.acknowledged
                && matches!(draft.phase, Phase::Composing)
                && store.participant(&draft.participant).is_some_and(|p| p.safety.is_stale())
        })
    }

    /// Continue chosen: the stale verification is acknowledged for this
    /// draft.
    pub fn acknowledge(&mut self, cx: &mut Context<Self>) {
        if let Some(draft) = &mut self.draft {
            draft.acknowledged = true;
            cx.notify();
        }
    }

    /// The sink the composer emits the first message through: scoped to
    /// the draft, which notes the submission and passes it on unchanged.
    pub fn sink(this: &Entity<Self>, root: &EventSink) -> EventSink {
        let weak = this.downgrade();
        root.scoped_map(Scope::Panel(SCOPE.to_owned()), move |event, _, cx| {
            if matches!(event.kind, UiEventKind::StartConversationRequested { .. }) {
                weak.update(cx, |draft, cx| draft.submitted(cx)).ok();
            }
            Some(event)
        })
    }

    fn submitted(&mut self, cx: &mut Context<Self>) {
        let since = self.store.read(cx).store().failures_total();
        if let Some(draft) = &mut self.draft {
            draft.phase = Phase::Sending { since };
            cx.notify();
        }
    }

    /// Follows the store: the conversation appeared, or its start failed.
    fn observe(&mut self, cx: &mut Context<Self>) {
        let Some(draft) = self.draft.clone() else { return };
        let store = self.store.read(cx).store();
        if store.context(&draft.context_id).is_some() {
            self.draft = None;
            cx.emit(NewConversationEvent::Started(draft.context_id));
            cx.notify();
            return;
        }
        let Phase::Sending { since } = draft.phase else {
            // A refusal reason may have changed (the contact accepted).
            cx.notify();
            return;
        };
        if let Some(failure) = store
            .failures_since(since)
            .iter()
            .rev()
            .find(|f| f.method == methods::CONVERSATION_START)
        {
            let reason = i18n::backend_text(&failure.message);
            if let Some(draft) = &mut self.draft {
                draft.phase = Phase::Failed(reason);
            }
            cx.notify();
        }
    }
}

/// The contact's name as shown.
fn name(store: &Store, participant: &ParticipantId) -> String {
    store.participant(participant).map_or_else(|| participant.short(), |p| p.display.clone())
}

/// Why the policy refuses, with what to do about it.
fn refusal(store: &Store, participant: &ParticipantId, error: &IntentError) -> SharedString {
    let name = name(store, participant);
    let key = match error {
        IntentError::SelfParticipant(_) => "conversation.new.error.self",
        IntentError::ContactLinkState { link: ContactLink::Outgoing, .. } => {
            "conversation.new.error.not_accepted"
        }
        IntentError::ContactLinkState { link: ContactLink::Incoming, .. } => {
            "conversation.new.error.incoming"
        }
        IntentError::ContactLinkState { link: ContactLink::Blocked, .. } => {
            "conversation.new.error.blocked"
        }
        IntentError::ContactLinkState { .. } | IntentError::UnknownParticipant(_) => {
            "conversation.new.error.not_a_contact"
        }
        IntentError::SafetyUnresolved(_) => "conversation.new.error.key_changed",
        IntentError::NotReachable(_) => "conversation.new.error.unreachable",
        other => return SharedString::from(other.to_string()),
    };
    tr_args(key, &[("name", &name)])
}

impl Render for NewConversation {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ack = self.awaits_acknowledgement(self.store.read(cx).store()).then(|| {
            let store = self.store.read(cx).store();
            let participant = self.draft.as_ref().map(|d| d.participant.clone());
            let participant = participant.unwrap_or_else(|| ParticipantId::new(""));
            let prompt =
                tr_args("identity.ack.conversation", &[("name", &name(store, &participant))]);
            let sink = self.sink.clone();
            crate::identity::stale_ack(
                "new-conversation-stale-ack",
                prompt,
                Box::new(move |_, window, cx| {
                    if let Some(sink) = &sink {
                        sink.emit(
                            UiEventKind::VerifySafetyPressed {
                                participant_id: participant.clone(),
                            },
                            window,
                            cx,
                        );
                    }
                }),
                Box::new(cx.listener(|this, _, _, cx| this.acknowledge(cx))),
                cx,
            )
        });
        let store = self.store.read(cx).store();
        let theme = cx.theme();
        let (muted, danger, border) = (theme.muted_foreground, theme.danger, theme.border);
        let title = self.draft.as_ref().map_or_else(
            || tr("conversation.new.title_unknown"),
            |d| tr_args("conversation.new.title", &[("name", &name(store, &d.participant))]),
        );
        let safety = self.draft.as_ref().and_then(|d| store.participant(&d.participant));
        let blocked = self.blocked(store);
        let (status, failed) = match self.draft.as_ref().map(|d| &d.phase) {
            Some(Phase::Failed(reason)) => {
                (Some(tr_args("conversation.new.failed", &[("reason", reason)])), true)
            }
            Some(Phase::Sending { .. }) => (Some(tr("conversation.new.sending")), false),
            // The acknowledgement says it all; no second line.
            _ if ack.is_some() => (None, false),
            _ => (blocked.clone(), blocked.is_some()),
        };
        v_flex()
            .id("new-conversation")
            .test_support()
            .flex_1()
            .min_h_0()
            .child(
                h_flex()
                    .id("new-conversation-header")
                    .gap_2()
                    .px_4()
                    .py_3()
                    .border_b_1()
                    .border_color(border)
                    .child(Icon::new(IconName::User).small())
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_weight(gpui_kit::FontWeight::MEDIUM)
                            .child(title),
                    )
                    .children(
                        safety.map(|p| {
                            div().text_xs().text_color(muted).child(i18n::safety(p.safety))
                        }),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .px_6()
                    .child(
                        div()
                            .text_sm()
                            .text_color(muted)
                            .text_center()
                            .child(tr("conversation.new.hint")),
                    )
                    .children(ack)
                    .children(status.map(|text| {
                        div()
                            .id("new-conversation-status")
                            .role(Role::Status)
                            .aria_label(text.clone())
                            .test_support()
                            .text_sm()
                            .text_center()
                            .text_color(if failed { danger } else { muted })
                            .child(text)
                    })),
            )
    }
}
