// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! A contact's profile (design F family): who it is and how far that is
//! proven, an Encryption section (derived design D-M1-2), Message and Call,
//! the conversations shared with it, and Block or Remove (both revoke what
//! was shared and need the host-drawn confirmation, which policy opens).
//!
//! The Encryption section says how far the identity is trusted ("Verified",
//! the neutral first-seen line, or a warning with Review while a change
//! waits; Review opens the identity change sheet) and, when a direct
//! conversation exists, how it is encrypted and with how many of the
//! contact's devices. Nothing is compared and nothing is marked verified
//! here.

use crate::style::DesignButton as _;
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::StyledExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, Context, Entity, EventEmitter, IntoElement, Render, Role, SharedString,
    Subscription, Window, div,
};
use ramflux_desktop_core::{
    ContactAction, ContextKind, IdentityState, Intent, IntentKind, Participant, ParticipantId,
    Scope, Store, UiEventKind, selectors,
};
use ramflux_desktop_navigation::direction::h_flex;
use ramflux_desktop_navigation::palette;
use ramflux_desktop_navigation::scale::{design_px, space, text_caption, text_secondary};

use crate::app::{StoreEvent, StoreHandle};
use crate::bubble::EventSink;
use crate::i18n::{self, tr, tr_args};
use crate::workspace::docks::contacts::rows::{action_label, trust_line};

/// What the profile asks the workspace to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContactProfileEvent {
    /// Show (or start) the direct conversation with this contact.
    Message(ParticipantId),
}

/// The profile's view state: which contact.
pub struct ContactProfile {
    store: Entity<StoreHandle>,
    sink: EventSink,
    participant: Option<ParticipantId>,
    _subscription: Subscription,
}

impl EventEmitter<ContactProfileEvent> for ContactProfile {}

impl ContactProfile {
    /// A profile reading `store`, emitting through `sink` (the root sink).
    pub fn new(store: Entity<StoreHandle>, sink: EventSink, cx: &mut Context<Self>) -> Self {
        let subscription = cx.subscribe(&store, |this, _, event: &StoreEvent, cx| {
            if let StoreEvent::Changed(changes) = event {
                if changes.is_reset() {
                    this.participant = None;
                }
                if changes.touches_docks() || changes.touches_status() {
                    cx.notify();
                }
            }
        });
        let sink = sink.scoped(Scope::Panel("section-contacts".into()));
        Self { store, sink, participant: None, _subscription: subscription }
    }

    /// Shows `participant` (or nothing). Opening a contact reads its
    /// identity state, and the security of the direct conversation with it.
    pub fn set_participant(&mut self, participant: Option<ParticipantId>, cx: &mut Context<Self>) {
        if self.participant != participant {
            self.participant = participant;
            if let Some(id) = self.participant.clone() {
                self.read(&id, cx);
            }
            cx.notify();
        }
    }

    /// Reads what the Encryption section shows.
    fn read(&self, participant: &ParticipantId, cx: &mut Context<Self>) {
        let context = direct_context(self.store.read(cx).store(), participant);
        self.store.update(cx, |store, cx| {
            store.dispatch(
                Intent::human(IntentKind::ReadIdentityState {
                    participant_id: participant.clone(),
                }),
                cx,
            );
            if let Some(context_id) = context {
                store.dispatch(
                    Intent::human(IntentKind::ReadConversationSecurity { context_id }),
                    cx,
                );
            }
        });
    }

    /// The shown contact.
    pub fn participant(&self) -> Option<&ParticipantId> {
        self.participant.as_ref()
    }

    /// The Encryption section: the identity line, then the conversation's
    /// security line and device count when a direct conversation exists.
    fn encryption_block(&self, contact: &Participant, cx: &mut Context<Self>) -> AnyElement {
        let security = {
            let store = self.store.read(cx).store();
            direct_context(store, &contact.id).and_then(|context| store.security(&context).cloned())
        };
        let (icon, color, state) = match contact.identity {
            IdentityState::Verified => {
                (Some(IconName::CircleCheck), cx.theme().success, tr("identity.state.verified"))
            }
            IdentityState::Tofu => (None, cx.theme().muted_foreground, tr("identity.state.tofu")),
            IdentityState::Changed => (
                Some(IconName::TriangleAlert),
                crate::tone::text(cx.theme().warning, cx),
                tr("identity.state.changed"),
            ),
        };
        let review = contact.identity.is_changed().then(|| {
            let (sink, participant_id) = (self.sink.clone(), contact.id.clone());
            Button::new("contact-review")
                .small()
                .design_secondary(cx)
                .design_label(tr("identity.changed.review"))
                .on_click(move |_, window, cx| {
                    sink.emit(
                        UiEventKind::ReviewIdentityPressed {
                            participant_id: participant_id.clone(),
                        },
                        window,
                        cx,
                    );
                })
        });
        v_flex()
            .id("contact-encryption")
            .test_support()
            .gap(space(2.))
            .child(div().font_medium().child(tr("security.section.title")))
            .child(
                h_flex()
                    .id("contact-identity-state")
                    .role(Role::Status)
                    .aria_label(state.clone())
                    .test_support()
                    .gap(space(2.))
                    .text_size(text_secondary())
                    .text_color(color)
                    .children(icon.map(|icon| Icon::new(icon).small()))
                    .child(div().child(state))
                    .children(review),
            )
            .children(security.map(|security| {
                h_flex()
                    .id("contact-security")
                    .test_support()
                    .gap(space(2.))
                    .flex_wrap()
                    .text_size(text_secondary())
                    .child(
                        div()
                            .id("contact-security-state")
                            .child(i18n::security_state(security.state)),
                    )
                    .when(security.peer_devices > 0, |line| {
                        line.child(div().text_color(cx.theme().muted_foreground).child("·")).child(
                            div().id("contact-security-devices").child(i18n::tr_count(
                                "security.devices",
                                u64::from(security.peer_devices),
                            )),
                        )
                    })
                    .children(
                        security
                            .any_ai
                            .then(|| crate::identity::ai_mark("contact-security-ai", cx)),
                    )
            }))
            .into_any_element()
    }
}

/// The direct conversation with `participant`, if there is one.
fn direct_context(
    store: &Store,
    participant: &ParticipantId,
) -> Option<ramflux_desktop_core::ContextId> {
    selectors::contexts_with(store, participant)
        .into_iter()
        .find(|c| c.kind == ContextKind::Direct)
        .map(|c| c.id.clone())
}

impl Render for ContactProfile {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(id) = self.participant.clone() else {
            return super::directory::placeholder(
                "contact-profile-empty",
                tr("sections.contact.choose"),
                tr("sections.contact.choose_help"),
                cx,
            );
        };
        let Some(contact) = self.store.read(cx).store().participant(&id).cloned() else {
            return super::directory::placeholder(
                "contact-profile-gone",
                tr("sections.contact.gone"),
                tr("sections.contact.gone_help"),
                cx,
            );
        };
        let (direct, call_reason, shared, block_reason, remove_reason) = {
            let store = self.store.read(cx).store();
            let direct = direct_context(store, &id);
            let call_reason = match &direct {
                Some(context) => crate::workspace::call_blocked(store, context),
                None => Some(tr("sections.contact.call_needs_conversation")),
            };
            let shared: Vec<(ramflux_desktop_core::ContextId, String)> =
                selectors::contexts_with(store, &id)
                    .into_iter()
                    .filter(|c| c.kind != ContextKind::Direct)
                    .map(|c| (c.id.clone(), selectors::context_title(store, c)))
                    .collect();
            (
                direct,
                call_reason,
                shared,
                crate::gate::reason(store, ContactAction::Block.method()),
                crate::gate::reason(store, ContactAction::Remove.method()),
            )
        };
        let theme = palette(cx);
        let unreachable = (contact.reachable == Some(false))
            .then(|| tr_args("dock.contacts.unreachable_reason", &[("name", &contact.display)]));
        let message_id = id.clone();
        let message = crate::gate::gated(
            Button::new("contact-message")
                .small()
                .design_primary(cx)
                .icon(IconName::MessageSquare)
                .design_label(tr("dock.contacts.message"))
                .on_click(cx.listener(move |_, _, _, cx| {
                    cx.emit(ContactProfileEvent::Message(message_id.clone()));
                })),
            unreachable,
            cx,
        );
        let call_sink = direct.map(|context| self.sink.scoped(Scope::Context(context)));
        // The same gate as the header's Call button: a contact whose identity
        // change waits opens the identity change sheet.
        let changed = contact.identity.is_changed().then(|| contact.id.clone());
        let call = crate::gate::gated(
            Button::new("contact-call")
                .small()
                .design_secondary(cx)
                .icon(IconName::Phone)
                .design_label(tr("sections.contact.call"))
                .on_click(move |event, window, cx| {
                    if let Some(sink) = &call_sink {
                        crate::identity::call_gate(changed.clone(), sink.clone())(
                            event, window, cx,
                        );
                    }
                }),
            call_reason,
            cx,
        );
        let gate_palette = ramflux_desktop_navigation::palette(cx);
        let danger_quiet =
            crate::style::DesignStyle::new(crate::style::ButtonKind::DangerQuiet, cx);
        let action =
            |button_id: &'static str, action: ContactAction, reason: Option<SharedString>| {
                let sink = self.sink.clone();
                let participant_id = id.clone();
                crate::gate::gated_in(
                    Button::new(button_id)
                        .small()
                        .design_style(&danger_quiet)
                        .design_label(action_label(action))
                        .on_click(move |_, window, cx| {
                            sink.emit(
                                UiEventKind::ContactActionPressed {
                                    participant_id: participant_id.clone(),
                                    action,
                                },
                                window,
                                cx,
                            );
                        }),
                    reason,
                    &gate_palette,
                )
            };
        let encryption = self.encryption_block(&contact, cx);
        v_flex()
            .id("contact-profile")
            .test_support()
            .role(Role::Region)
            .aria_label(tr_args("sections.contact.a11y", &[("name", &contact.display)]))
            .size_full()
            .min_w_0()
            .overflow_y_scroll()
            .child(
                div().border_b_1().border_color(cx.theme().border).child(
                    crate::page_header::page_header(
                        ramflux_desktop_navigation::row::list_text(&contact.display),
                        ramflux_desktop_navigation::scale::style::subheading(),
                        SharedString::from(contact.id.to_string()),
                        Some(
                            crate::avatar::avatar(
                                ramflux_desktop_navigation::row::initial(&contact.display),
                                super::contacts::avatar_kind(contact.kind),
                                theme,
                            )
                            .into_any_element(),
                        ),
                        h_flex().gap(space(2.)).flex_wrap().child(message).child(call),
                        theme,
                    ),
                ),
            )
            .child(
                v_flex()
                    .gap(space(4.))
                    .p(space(4.))
                    .max_w(design_px(theme.column_max))
                    .children(crate::identity::name_warning("contact-name-warning", &contact, cx))
                    .child(
                        h_flex()
                            .gap(space(2.))
                            .flex_wrap()
                            .children(crate::identity::kind_mark(
                                "contact-kind",
                                contact.kind_claim,
                                cx,
                            ))
                            .children(trust_line(&contact).map(|line| {
                                div()
                                    .id("contact-trust")
                                    .test_support()
                                    .text_size(text_caption())
                                    .text_color(cx.theme().muted_foreground)
                                    .child(line)
                            })),
                    )
                    .child(encryption)
                    .child(
                        v_flex()
                            .id("contact-shared")
                            .test_support()
                            .gap(space(1.))
                            .child(div().font_medium().child(tr("sections.contact.shared")))
                            .when(shared.is_empty(), |col| {
                                col.child(
                                    div()
                                        .text_size(text_secondary())
                                        .text_color(cx.theme().muted_foreground)
                                        .child(tr("sections.contact.shared_none")),
                                )
                            })
                            .children(shared.into_iter().map(|(context, title)| {
                                div()
                                    .id(crate::ids::domain_id("contact-shared", &context))
                                    .text_size(text_secondary())
                                    .child(ramflux_desktop_navigation::row::list_text(&title))
                            })),
                    )
                    .child(
                        v_flex()
                            .id("contact-danger")
                            .test_support()
                            .gap(space(2.))
                            .child(
                                div()
                                    .text_size(text_caption())
                                    .text_color(cx.theme().muted_foreground)
                                    .whitespace_normal()
                                    .child(tr("sections.contact.danger_help")),
                            )
                            .child(
                                h_flex()
                                    .gap(space(2.))
                                    .child(action(
                                        "contact-block",
                                        ContactAction::Block,
                                        block_reason,
                                    ))
                                    .child(action(
                                        "contact-remove",
                                        ContactAction::Remove,
                                        remove_reason,
                                    )),
                            ),
                    ),
            )
            .into_any_element()
    }
}
