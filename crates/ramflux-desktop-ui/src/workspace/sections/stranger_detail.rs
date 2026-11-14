// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! One stranger's request (design F17): who asks and how little is proven
//! (first contact, identity not verified), their note, the messages held
//! until a decision, the safety number to compare, and what accepting does.
//!
//! The three fixed actions sit in the design's order in a bottom bar:
//! Decline, Reply, Accept. Reply stays visible and disabled with the typed
//! reason until the backend can reply without accepting (F21); it never
//! accepts. Block (host-drawn confirmation, policy opens it) and Report
//! (unavailable the same way) are icon buttons at the header's end, which
//! the board leaves free.

use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, StyledExt as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Context, Role, SharedString, Window, div};
use ramflux_desktop_core::{RequestAnswer, UiEventKind};
use ramflux_desktop_navigation::generated::catalog;
use ramflux_desktop_navigation::palette;
use ramflux_desktop_sdk::business::ContractGap;

use super::directory;
use super::strangers::StrangersArea;
use crate::i18n::{self, tr, tr_args};
use crate::style::DesignButton as _;

impl StrangersArea {
    /// The detail pane of the chosen request.
    pub fn detail(&mut self, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let request = {
            let store = self.store.read(cx).store();
            self.selected
                .as_deref()
                .and_then(|id| store.contact_request_by_id(id))
                .filter(|request| request.awaits_answer())
                .cloned()
        };
        let Some(request) = request else {
            return directory::placeholder(
                "stranger-detail-empty",
                tr("sections.strangers.choose"),
                tr("sections.strangers.choose_help"),
                cx,
            );
        };
        let (participant, answer_reason, expired) = {
            let handle = self.store.read(cx);
            let store = handle.store();
            let now = handle.session().map(|session| session.now());
            (
                store.participant(&request.peer).cloned(),
                crate::workspace::docks::contacts::requests::reason(store),
                request.expires_at.zip(now).is_some_and(|(at, now)| at <= now),
            )
        };
        let theme = palette(cx);
        let name: SharedString =
            participant.as_ref().map_or_else(|| request.peer.short(), |p| p.display.clone()).into();
        let reason = if expired { Some(tr("sections.strangers.expired")) } else { answer_reason };
        let answer = |id: &'static str, answer: RequestAnswer, label: &str| {
            let sink = self.sink.clone();
            let request_id = request.id.clone();
            let button = Button::new(id)
                .label(tr(label))
                .accessibility_label(tr_args(&format!("{label}_for"), &[("name", &name)]))
                .on_click(move |_, window, cx| {
                    sink.emit(
                        UiEventKind::ContactRequestAnswered {
                            request_id: request_id.clone(),
                            answer,
                        },
                        window,
                        cx,
                    );
                });
            let button = match answer {
                RequestAnswer::Accept => button.design_primary(cx),
                RequestAnswer::Decline => button.design_secondary(cx),
                RequestAnswer::Block => {
                    button.small().ghost().icon(IconName::Ban).tooltip(tr(label))
                }
            };
            crate::gate::gated(button, reason.clone())
        };
        let reply = crate::gate::gated(
            Button::new("stranger-reply")
                .design_secondary(cx)
                .label(tr("sections.strangers.reply")),
            Some(directory::unavailable(ContractGap::StrangerActions)),
        );
        let report = crate::gate::gated(
            Button::new("stranger-report")
                .small()
                .ghost()
                .icon(IconName::Flag)
                .tooltip(tr("sections.strangers.report"))
                .accessibility_label(tr("sections.strangers.report")),
            Some(directory::unavailable(ContractGap::StrangerActions)),
        );
        let unverified = participant
            .as_ref()
            .is_none_or(|p| p.safety != ramflux_desktop_core::SafetyState::Verified);
        let handle = handle(&request.peer);
        let host = node_host(&request.peer);
        let warning_text = crate::tone::text(theme.warning, cx);
        let column = |el: gpui_kit::Div| el.w_full().max_w(rem(theme.column_max)).mx_auto();
        // The header (board F17): avatar, name with the "Not a contact" chip, the
        // handle; Block and Report sit at its end.
        let header = h_flex()
            .id("stranger-header")
            .test_support()
            .flex_none()
            .h(rem(88.0))
            .px(rem(32.0))
            .gap_3()
            .border_b_1()
            .border_color(theme.line)
            .child(catalog::avatar(
                ramflux_desktop_navigation::row::initial(&name),
                catalog::AvatarKind::Person,
                theme,
            ))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                div()
                                    .text_size(rem(theme.size_subheading))
                                    .font_semibold()
                                    .truncate()
                                    .child(ramflux_desktop_navigation::row::list_text(&name)),
                            )
                            .child(warning_chip(
                                "stranger-not-contact",
                                tr("sections.strangers.not_contact"),
                                cx,
                            )),
                    )
                    .child(
                        div()
                            .text_size(rem(theme.size_secondary))
                            .text_color(theme.secondary)
                            .truncate()
                            .child(handle.clone()),
                    ),
            )
            .child(
                h_flex()
                    .gap_1()
                    .child(answer(
                        "stranger-block",
                        RequestAnswer::Block,
                        "dock.contacts.request.block",
                    ))
                    .child(report),
            );
        let banner_fact = |label: SharedString, value: SharedString| {
            v_flex()
                .gap_1()
                .child(
                    div().text_size(rem(theme.size_meta)).text_color(theme.secondary).child(label),
                )
                .child(
                    div()
                        .text_size(rem(theme.size_secondary))
                        .font_family(cx.theme().mono_font_family.clone())
                        .child(value),
                )
        };
        let safety: SharedString = request
            .safety_number
            .as_deref()
            .map_or_else(|| tr("sections.contact.no_number"), |n| group_digits(n).into());
        // The warning bar: first contact, unverified, and what to compare.
        let banner =
            h_flex()
                .id("stranger-unverified")
                .test_support()
                .items_start()
                .max_w(rem(560.0))
                .gap_3()
                .p(rem(12.0))
                .rounded(rem(theme.radius_card))
                .bg(theme.warning_bg)
                .child(Icon::new(IconName::ShieldAlert).small().text_color(warning_text))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_2()
                        .child(div().text_size(rem(theme.size_secondary)).font_semibold().child(
                            if unverified {
                                tr("sections.strangers.unverified")
                            } else {
                                tr("sections.strangers.not_contact")
                            },
                        ))
                        .child(
                            h_flex()
                                .flex_wrap()
                                .gap(rem(24.0))
                                .child(div().id("stranger-safety").test_support().child(
                                    banner_fact(tr("sections.contact.safety_number"), safety),
                                ))
                                .children(host.map(|host| {
                                    banner_fact(
                                        tr_args("sections.strangers.node", &[("host", &host)]),
                                        SharedString::default(),
                                    )
                                })),
                        ),
                );
        let bubble = |id: &'static str, text: SharedString| {
            h_flex().child(catalog::message_bubble(
                catalog::MessageContent::new(
                    id.into(),
                    catalog::BubbleKind::Peer,
                    div().whitespace_normal().child(text),
                ),
                theme,
            ))
        };
        let body = v_flex()
            .id("stranger-body")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .py(rem(24.0))
            .px(rem(32.0))
            .child(
                column(v_flex())
                    .gap_3()
                    .child(banner)
                    .children(participant.as_ref().and_then(|p| {
                        crate::identity::name_warning("stranger-name-warning", p, cx)
                    }))
                    .children(request.note.clone().map(|note| {
                        div().id("stranger-note").test_support().child(bubble(
                            "stranger-note-bubble",
                            tr_args("sections.strangers.note_prefix", &[("note", &note)]),
                        ))
                    }))
                    .when(request.held_messages > 0, |col| {
                        col.child(
                            div().id("stranger-held").test_support().child(
                                match request.preview.clone() {
                                    Some(preview) => bubble("stranger-held-bubble", preview.into())
                                        .into_any_element(),
                                    None => div()
                                        .text_size(rem(theme.size_secondary))
                                        .text_color(theme.secondary)
                                        .child(tr_args(
                                            "sections.strangers.held",
                                            &[("count", &request.held_messages.to_string())],
                                        ))
                                        .child(tr("sections.strangers.held_hidden"))
                                        .into_any_element(),
                                },
                            ),
                        )
                    })
                    .children(request.expires_at.map(|at| {
                        div()
                            .id("stranger-expires")
                            .test_support()
                            .text_size(rem(theme.size_secondary))
                            .text_color(theme.secondary)
                            .child(i18n::join(
                                &tr("sections.strangers.expires"),
                                &i18n::date_time(at),
                            ))
                    }))
                    .child(
                        div()
                            .text_size(rem(theme.size_meta))
                            .text_color(theme.secondary)
                            .whitespace_normal()
                            .child(tr("sections.strangers.reply_help")),
                    ),
            );
        // The fixed bottom bar: what accepting does, then the three
        // actions in the board's order.
        let bar = h_flex()
            .flex_none()
            .h(rem(64.0))
            .px(rem(32.0))
            .border_t_1()
            .border_color(theme.line)
            .child(
                column(h_flex())
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(rem(theme.size_secondary))
                            .text_color(theme.secondary)
                            .truncate()
                            .child(tr("sections.strangers.bar_help")),
                    )
                    .child(
                        h_flex()
                            .id("stranger-actions")
                            .test_support()
                            .gap_2()
                            .child(answer(
                                "stranger-decline",
                                RequestAnswer::Decline,
                                "dock.contacts.request.decline",
                            ))
                            .child(reply)
                            .child(answer(
                                "stranger-accept",
                                RequestAnswer::Accept,
                                "dock.contacts.request.accept",
                            )),
                    ),
            );
        v_flex()
            .id("stranger-detail")
            .test_support()
            .role(Role::Region)
            .aria_label(tr_args("sections.strangers.detail_a11y", &[("name", &name)]))
            .size_full()
            .min_w_0()
            .child(header)
            .child(body)
            .child(bar)
            .into_any_element()
    }
}

fn rem(px: f32) -> gpui_kit::Rems {
    gpui_kit::rems(px / 15.0)
}

/// The handle shown for `peer`: `@user@host` when the id is an address,
/// else the id itself.
pub(super) fn handle(peer: &ramflux_desktop_core::ParticipantId) -> SharedString {
    let id = peer.as_str();
    if id.contains('@') && !id.starts_with('@') {
        format!("@{id}").into()
    } else {
        id.to_owned().into()
    }
}

/// The node of `peer` when its id is an address (`user@host`).
fn node_host(peer: &ramflux_desktop_core::ParticipantId) -> Option<String> {
    peer.as_str().rsplit_once('@').map(|(_, host)| host.to_owned()).filter(|h| !h.is_empty())
}

/// A safety number in groups of four digits, as the board prints it.
fn group_digits(number: &str) -> String {
    let digits: Vec<char> = number.chars().filter(|c| !c.is_whitespace()).collect();
    digits.chunks(4).map(|chunk| chunk.iter().collect::<String>()).collect::<Vec<_>>().join(" ")
}

/// The warning chip ("Not a contact", "Identity not verified").
pub(crate) fn warning_chip(id: &'static str, text: SharedString, cx: &gpui_kit::App) -> AnyElement {
    let theme = palette(cx);
    div()
        .id(id)
        .test_support()
        .flex_none()
        .px_2()
        .py(rem(2.0))
        .rounded(rem(theme.radius_chip))
        .bg(theme.warning_bg)
        .text_color(crate::tone::text(theme.warning, cx))
        .text_size(rem(theme.size_meta))
        .child(text)
        .into_any_element()
}
