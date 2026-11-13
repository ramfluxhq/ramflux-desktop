// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! One stranger's request (design F17): who asks and how little is proven
//! (first contact, identity not verified), their note, the messages held
//! until a decision, the safety number to compare, and what accepting does.
//!
//! The three fixed actions sit in the design's order: Decline, Reply,
//! Accept. Reply stays visible and disabled with the typed reason until the
//! backend can reply without accepting (F21); it never accepts. Block keeps
//! the host-drawn confirmation (policy opens it). Report is unavailable the same way.

use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Context, Role, SharedString, Window, div};
use ramflux_desktop_core::{RequestAnswer, UiEventKind};
use ramflux_desktop_navigation::generated::catalog;
use ramflux_desktop_navigation::palette;
use ramflux_desktop_sdk::business::ContractGap;

use super::directory;
use super::strangers::StrangersArea;
use crate::i18n::{self, tr, tr_args};

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
                .small()
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
                RequestAnswer::Accept => button.primary(),
                RequestAnswer::Decline => button.outline(),
                RequestAnswer::Block => button.danger(),
            };
            crate::gate::gated(button, reason.clone())
        };
        let reply = crate::gate::gated(
            Button::new("stranger-reply")
                .small()
                .outline()
                .icon(IconName::Reply)
                .label(tr("sections.strangers.reply")),
            Some(directory::unavailable(ContractGap::StrangerActions)),
        );
        let report = crate::gate::gated(
            Button::new("stranger-report")
                .small()
                .ghost()
                .icon(IconName::Flag)
                .label(tr("sections.strangers.report")),
            Some(directory::unavailable(ContractGap::StrangerActions)),
        );
        let unverified = participant
            .as_ref()
            .is_none_or(|p| p.safety != ramflux_desktop_core::SafetyState::Verified);
        let fact = |id: &'static str, label: SharedString, value: SharedString| {
            v_flex()
                .id(id)
                .test_support()
                .gap_1()
                .child(div().text_xs().text_color(cx.theme().muted_foreground).child(label))
                .child(div().text_sm().whitespace_normal().child(value))
        };
        v_flex()
            .id("stranger-detail")
            .test_support()
            .role(Role::Region)
            .aria_label(tr_args("sections.strangers.detail_a11y", &[("name", &name)]))
            .size_full()
            .min_w_0()
            .overflow_y_scroll()
            .child(div().border_b_1().border_color(cx.theme().border).child(catalog::page_header(
                ramflux_desktop_navigation::row::list_text(&name),
                SharedString::from(request.peer.to_string()),
                false,
                catalog::avatar(
                    ramflux_desktop_navigation::row::initial(&name),
                    catalog::AvatarKind::Person,
                    theme,
                ),
                div(),
                theme,
            )))
            .child(
                v_flex()
                    .gap_4()
                    .p_4()
                    .max_w(gpui_kit::rems(theme.column_max / 16.0))
                    .when(unverified, |col| {
                        col.child(
                            h_flex()
                                .id("stranger-unverified")
                                .test_support()
                                .gap_2()
                                .text_sm()
                                .text_color(crate::tone::text(cx.theme().warning, cx))
                                .child(Icon::new(IconName::ShieldAlert).small())
                                .child(tr("sections.strangers.unverified")),
                        )
                    })
                    .children(participant.as_ref().and_then(|p| {
                        crate::identity::name_warning("stranger-name-warning", p, cx)
                    }))
                    .children(request.note.clone().map(|note| {
                        fact("stranger-note", tr("sections.strangers.note"), note.into())
                    }))
                    .when(request.held_messages > 0, |col| {
                        col.child(fact(
                            "stranger-held",
                            tr_args(
                                "sections.strangers.held",
                                &[("count", &request.held_messages.to_string())],
                            ),
                            request
                                .preview
                                .clone()
                                .map_or_else(|| tr("sections.strangers.held_hidden"), Into::into),
                        ))
                    })
                    .child(fact(
                        "stranger-safety",
                        tr("sections.contact.safety_number"),
                        request
                            .safety_number
                            .clone()
                            .map_or_else(|| tr("sections.contact.no_number"), Into::into),
                    ))
                    .children(request.expires_at.map(|at| {
                        fact(
                            "stranger-expires",
                            tr("sections.strangers.expires"),
                            i18n::date_time(at).into(),
                        )
                    }))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .whitespace_normal()
                            .child(tr("sections.strangers.accept_help")),
                    )
                    .child(
                        h_flex()
                            .id("stranger-actions")
                            .test_support()
                            .flex_wrap()
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
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .whitespace_normal()
                            .child(tr("sections.strangers.reply_help")),
                    )
                    .child(
                        h_flex()
                            .flex_wrap()
                            .gap_2()
                            .child(answer(
                                "stranger-block",
                                RequestAnswer::Block,
                                "dock.contacts.request.block",
                            ))
                            .child(report),
                    ),
            )
            .into_any_element()
    }
}
