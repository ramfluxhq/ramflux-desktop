// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! A contact's profile (design F family): who it is and how far that is
//! proven, the safety number and its comparison, Message and Call, the
//! conversations shared with it, and Block or Remove (both revoke what was
//! shared and need the host hold, which policy opens).
//!
//! Verifying never happens on one click: Compare opens the number and what
//! to compare it with, and only "The numbers match" emits the verification.
//! A changed or forked key shows a sticky danger banner until resolved.

use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::StyledExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Context, Entity, EventEmitter, IntoElement, Render, Role, SharedString,
    Subscription, Window, div,
};
use ramflux_desktop_core::{
    ContactAction, ContextKind, Participant, ParticipantId, SafetyState, Scope, Store, UiEventKind,
    selectors,
};
use ramflux_desktop_navigation::generated::catalog;
use ramflux_desktop_navigation::palette;

use crate::app::{StoreEvent, StoreHandle};
use crate::bubble::EventSink;
use crate::i18n::{tr, tr_args};
use crate::workspace::docks::contacts::rows::{action_label, safety_label, trust_line};

/// What the profile asks the workspace to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContactProfileEvent {
    /// Show (or start) the direct conversation with this contact.
    Message(ParticipantId),
}

/// The profile's view state: which contact, and whether the safety number
/// comparison is open.
pub struct ContactProfile {
    store: Entity<StoreHandle>,
    sink: EventSink,
    participant: Option<ParticipantId>,
    comparing: bool,
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
                    this.comparing = false;
                }
                if changes.touches_docks() || changes.touches_status() {
                    cx.notify();
                }
            }
        });
        let sink = sink.scoped(Scope::Panel("section-contacts".into()));
        Self { store, sink, participant: None, comparing: false, _subscription: subscription }
    }

    /// Shows `participant` (or nothing). A new contact closes a comparison.
    pub fn set_participant(&mut self, participant: Option<ParticipantId>, cx: &mut Context<Self>) {
        if self.participant != participant {
            self.participant = participant;
            self.comparing = false;
            cx.notify();
        }
    }

    /// The shown contact.
    pub fn participant(&self) -> Option<&ParticipantId> {
        self.participant.as_ref()
    }

    fn emit(&self, kind: UiEventKind, window: &mut Window, cx: &mut App) {
        self.sink.emit(kind, window, cx);
    }

    fn safety_block(&self, contact: &Participant, cx: &mut Context<Self>) -> AnyElement {
        let theme = palette(cx);
        let number = contact.safety_number.clone();
        let grouped: Option<SharedString> = number.as_ref().map(|n| group_digits(n).into());
        let state = safety_label(contact.safety);
        let danger = matches!(contact.safety, SafetyState::KeyChanged | SafetyState::Forked);
        let id = contact.id.clone();
        let mut block = v_flex()
            .id("contact-safety")
            .test_support()
            .gap_2()
            .p_3()
            .rounded(cx.theme().radius_lg)
            .border_1()
            .border_color(if danger { cx.theme().danger } else { cx.theme().border })
            .child(
                h_flex()
                    .gap_2()
                    .child(Icon::new(IconName::ShieldCheck).small())
                    .child(div().font_medium().child(tr("sections.contact.safety_number")))
                    .child(div().flex_1())
                    .child(
                        div()
                            .id("contact-safety-state")
                            .test_support()
                            .text_xs()
                            .text_color(if danger {
                                cx.theme().danger
                            } else {
                                cx.theme().muted_foreground
                            })
                            .child(state),
                    ),
            )
            .child(
                div()
                    .id("contact-safety-number")
                    .test_support()
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_sm()
                    .whitespace_normal()
                    .child(grouped.clone().unwrap_or_else(|| tr("sections.contact.no_number"))),
            );
        if danger {
            block = block.child(catalog::trust_banner(
                catalog::NoticeContent::new(
                    tr("sections.contact.key_changed"),
                    tr("sections.contact.key_changed_help"),
                    Icon::new(IconName::ShieldAlert).text_color(cx.theme().danger),
                    div(),
                ),
                true,
                theme,
            ));
        }
        if contact.safety == SafetyState::Verified {
            return block.into_any_element();
        }
        if self.comparing {
            let help = tr_args("dock.contacts.verify.compare", &[("name", &contact.display)]);
            // Without a number there is nothing to compare and nothing to confirm.
            let blocked = number
                .is_none()
                .then(|| tr_args("dock.contacts.verify.no_number", &[("name", &contact.display)]));
            block = block
                .child(
                    div()
                        .id("contact-compare-help")
                        .test_support()
                        .role(Role::Status)
                        .aria_label(help.clone())
                        .text_xs()
                        .whitespace_normal()
                        .child(help),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(crate::gate::gated(
                            Button::new("contact-numbers-match")
                                .small()
                                .primary()
                                .label(tr("dock.contacts.verify.confirm"))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.comparing = false;
                                    this.emit(
                                        UiEventKind::VerifySafetyPressed {
                                            participant_id: id.clone(),
                                        },
                                        window,
                                        cx,
                                    );
                                    cx.notify();
                                })),
                            blocked,
                        ))
                        .child(
                            Button::new("contact-compare-cancel")
                                .small()
                                .ghost()
                                .label(tr("dock.contacts.verify.cancel"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.comparing = false;
                                    cx.notify();
                                })),
                        ),
                );
        } else {
            block = block.child(
                h_flex().child(
                    Button::new("contact-compare")
                        .small()
                        .outline()
                        .label(crate::identity::verify_label(contact.safety))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.comparing = true;
                            cx.notify();
                        })),
                ),
            );
        }
        block.into_any_element()
    }
}

/// The safety number in groups of four digits (spaces kept as given).
fn group_digits(number: &str) -> String {
    if number.contains(' ') {
        return number.to_owned();
    }
    let chars: Vec<char> = number.chars().collect();
    chars.chunks(4).map(|chunk| chunk.iter().collect::<String>()).collect::<Vec<_>>().join(" ")
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
                .primary()
                .icon(IconName::MessageSquare)
                .label(tr("dock.contacts.message"))
                .on_click(cx.listener(move |_, _, _, cx| {
                    cx.emit(ContactProfileEvent::Message(message_id.clone()));
                })),
            unreachable,
        );
        let call_sink = direct.map(|context| self.sink.scoped(Scope::Context(context)));
        let call = crate::gate::gated(
            Button::new("contact-call")
                .small()
                .outline()
                .icon(IconName::Phone)
                .label(tr("sections.contact.call"))
                .on_click(move |_, window, cx| {
                    if let Some(sink) = &call_sink {
                        sink.emit(UiEventKind::CallStartRequested, window, cx);
                    }
                }),
            call_reason,
        );
        let action =
            |button_id: &'static str, action: ContactAction, reason: Option<SharedString>| {
                let sink = self.sink.clone();
                let participant_id = id.clone();
                crate::gate::gated(
                    Button::new(button_id)
                        .small()
                        .outline()
                        .danger()
                        .label(action_label(action))
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
                )
            };
        let safety = self.safety_block(&contact, cx);
        v_flex()
            .id("contact-profile")
            .test_support()
            .role(Role::Region)
            .aria_label(tr_args("sections.contact.a11y", &[("name", &contact.display)]))
            .size_full()
            .min_w_0()
            .overflow_y_scroll()
            .child(div().border_b_1().border_color(cx.theme().border).child(catalog::page_header(
                ramflux_desktop_navigation::row::list_text(&contact.display),
                SharedString::from(contact.id.to_string()),
                false,
                catalog::avatar(
                    ramflux_desktop_navigation::row::initial(&contact.display),
                    super::contacts::avatar_kind(contact.kind),
                    theme,
                ),
                h_flex().gap_2().flex_wrap().child(message).child(call),
                theme,
            )))
            .child(
                v_flex()
                    .gap_4()
                    .p_4()
                    .max_w(gpui_kit::rems(theme.column_max / 16.0))
                    .children(crate::identity::name_warning("contact-name-warning", &contact, cx))
                    .child(
                        h_flex()
                            .gap_2()
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
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(line)
                            })),
                    )
                    .child(safety)
                    .child(
                        v_flex()
                            .id("contact-shared")
                            .test_support()
                            .gap_1()
                            .child(div().font_medium().child(tr("sections.contact.shared")))
                            .when(shared.is_empty(), |col| {
                                col.child(
                                    div()
                                        .text_sm()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(tr("sections.contact.shared_none")),
                                )
                            })
                            .children(shared.into_iter().map(|(context, title)| {
                                div()
                                    .id(crate::ids::domain_id("contact-shared", &context))
                                    .text_sm()
                                    .child(ramflux_desktop_navigation::row::list_text(&title))
                            })),
                    )
                    .child(
                        v_flex()
                            .id("contact-danger")
                            .test_support()
                            .gap_2()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .whitespace_normal()
                                    .child(tr("sections.contact.danger_help")),
                            )
                            .child(
                                h_flex()
                                    .gap_2()
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
