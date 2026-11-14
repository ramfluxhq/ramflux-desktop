// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Native row composition around the generated, presentation-only message shell.

use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _, ThemeStyled as _, h_flex,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, IntoElement, Role, TestSupportExt as _, Window, div, rems};
use ramflux_desktop_core::projection::messages::Mutation;
use ramflux_desktop_core::{MessagePayload, MomentKind, ParticipantKind, Receipt, UiEventKind};
use ramflux_desktop_navigation::{generated::catalog, palette};

use super::{MomentRow, RowState};
use crate::{
    bubble::EventSink,
    i18n::{self, tr},
    ids::domain_id,
};

// Kit's dropdown trait is implemented only for its styled Button. This local
// adapter lets Base own an explicit retained trigger focus handle.
#[derive(IntoElement)]
struct MenuTrigger(gpui_kit::base::Button);
impl Styled for MenuTrigger {
    fn style(&mut self) -> &mut gpui_kit::StyleRefinement {
        self.0.style()
    }
}
impl InteractiveElement for MenuTrigger {
    fn interactivity(&mut self) -> &mut gpui_kit::Interactivity {
        self.0.interactivity()
    }
}
impl gpui_kit::component::Selectable for MenuTrigger {
    fn is_selected(&self) -> bool {
        gpui_kit::component::Selectable::is_selected(&self.0)
    }
    fn selected(self, selected: bool) -> Self {
        Self(self.0.selected(selected))
    }
}
impl DropdownMenu for MenuTrigger {}
impl RenderOnce for MenuTrigger {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.0
    }
}

/// Group only consecutive live messages with the same actual author, context
/// and local day. A service record, tombstone or warning starts a fresh group.
pub(crate) fn joins(previous: &MomentRow, current: &MomentRow) -> bool {
    matches!((&previous.moment.kind, &current.moment.kind), (MomentKind::Message(a), MomentKind::Message(b)) if !a.tombstone && !b.tombstone)
        && previous.moment.origin == current.moment.origin
        && previous.moment.context_id == current.moment.context_id
        && current.author_warning.is_none()
        && current.moment.at.0.saturating_sub(previous.moment.at.0) <= 300_000
        && current.moment.at >= previous.moment.at
        && current.day_header.is_none()
}

pub(crate) fn render(
    row: &MomentRow,
    body: AnyElement,
    sink: &EventSink,
    state: &RowState,
    window: &mut Window,
    cx: &App,
) -> AnyElement {
    let MomentKind::Message(payload) = &row.moment.kind else {
        return body;
    };
    let theme = palette(cx);
    // Native inputs need an explicit measure; shape the actual stored lines
    // with the same font and size rather than stretching every short reply.
    let body_width = row.body.as_ref().filter(|_| state.message_text.is_some()).map(|body| {
        let style = window.text_style();
        // Measured in the face the body paints with (the window's text style
        // is not the shell's while rendering).
        let font = crate::assets::design_font(gpui_kit::FontWeight::NORMAL);
        // Bodies use the design's body size (15 px on a 15 px rem).
        let size = window.rem_size() * (theme.size_body / 15.0);
        let mut width = gpui_kit::px(0.0);
        let attachment = payload
            .attachment
            .as_ref()
            .and_then(|id| row.refs.objects.get(id))
            .and_then(|object| object.name.as_deref());
        let quote =
            row.refs.reply_excerpt.as_deref().map(ramflux_desktop_navigation::row::list_text);
        let author = ramflux_desktop_navigation::row::list_text(&row.author);
        let attachment = attachment.map(ramflux_desktop_navigation::row::list_text);
        for line in body
            .split('\n')
            .chain(std::iter::once(author.as_ref()))
            .chain(quote.as_deref())
            .chain(attachment.as_deref())
        {
            let line: gpui_kit::SharedString = line.to_owned().into();
            let run = gpui_kit::TextRun {
                len: line.len(),
                font: font.clone(),
                color: style.color,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            width = width.max(window.text_system().shape_line(line, size, &[run], None).width);
            if width >= window.rem_size() * (theme.bubble_max / 16.0) {
                break;
            }
        }
        // The time sits at the end of the body line (design `RF / Bubble`):
        // reserve its width and the 8 px gap before it.
        let time: gpui_kit::SharedString = i18n::time_of_day(row.moment.at).into();
        let run = gpui_kit::TextRun {
            len: time.len(),
            font: font.clone(),
            color: style.color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let meta = window
            .text_system()
            .shape_line(time, window.rem_size() * (theme.size_caption / 15.0), &[run], None)
            .width;
        // Shell padding plus a small caret/glyph rounding allowance. Metadata
        // and attachment/quote content retain their own intrinsic minimum.
        // The selectable body keeps Kit's 4 px editor inset on each side.
        width += window.rem_size() * (8.0 / 15.0);
        // An own message also shows its receipt icon (12 px and a 4 px gap).
        let receipt =
            if row.outbound { window.rem_size() * (16.0 / 15.0) } else { gpui_kit::px(0.) };
        (width + meta + receipt + window.rem_size() * (8.0 / 15.0) + window.rem_size() * 2.0)
            .max(window.rem_size() * 8.0)
    });
    let agent =
        row.refs.people.get(&row.moment.origin).is_some_and(|p| p.kind == ParticipantKind::Agent);
    let kind = if row.outbound {
        catalog::BubbleKind::Own
    } else if state.joined {
        catalog::BubbleKind::Joined
    } else {
        catalog::BubbleKind::Peer
    };
    let sender = if agent {
        crate::i18n::join(
            &ramflux_desktop_navigation::row::list_text(&row.author),
            &tr("message_ui.agent"),
        )
    } else {
        ramflux_desktop_navigation::row::list_text(&row.author)
    };
    let (lifecycle, tone) = super::state_label(row);
    let show_author = !state.joined || row.author_warning.is_some();
    let content = v_flex()
        .min_w_0()
        .gap_1()
        .when(show_author && !row.outbound, |col| {
            col.child(
                div()
                    .id("message-sender")
                    .test_support()
                    .aria_label(sender.clone())
                    .text_size(rems(theme.size_secondary / 15.0))
                    .font_semibold()
                    .child(sender),
            )
        })
        .when_some(row.author_warning.as_ref(), |col, (warning, mixed_only)| {
            col.child(crate::identity::author_warning(
                "author-name-warning",
                &row.moment.origin,
                warning,
                *mixed_only,
                cx,
            ))
        })
        .when_some(tone, |col, tone| {
            col.child(
                div()
                    .id("row-state")
                    .test_support()
                    .text_xs()
                    .text_color(crate::tone::text(tone.color(cx), cx))
                    .child(lifecycle),
            )
        })
        .child(
            h_flex()
                .w_full()
                .min_w_0()
                .items_end()
                .gap_2()
                .child(div().flex_1().min_w_0().child(body))
                .child(
                    h_flex()
                        .flex_none()
                        .gap_1()
                        .pb(gpui_kit::px(2.))
                        .text_size(rems(theme.size_caption / 15.0))
                        .text_color(if row.outbound {
                            theme.outgoing_meta
                        } else {
                            theme.incoming_meta
                        })
                        .child(i18n::time_of_day(row.moment.at))
                        .children(receipt_meta(payload, row.outbound)),
                ),
        );
    let shell = catalog::message_bubble(
        catalog::MessageContent::new(row.moment.id.to_string().into(), kind, content),
        theme,
    );
    let copy = sink.clone();
    let copy_enabled = !payload.tombstone && row.body.is_some();
    let own = row.outbound;
    let deleted = payload.tombstone;
    let menu_focus = state.menu_focus.clone();
    let trigger = gpui_kit::base::Button::new("message-menu")
        .when_some(menu_focus.as_ref(), |button, focus| button.track_focus(focus))
        .flex()
        .items_center()
        .justify_center()
        .flex_none()
        .size(rems(24.0 / 16.0))
        .rounded(cx.theme().radius)
        .border_1()
        .border_color(cx.theme().transparent)
        .hover(|style| style.bg(cx.theme().secondary_hover))
        .focus_visible(|style| style.border_color(cx.theme().ring))
        .accessibility_label(tr("message_ui.actions"))
        .child(Icon::new(IconName::Ellipsis).small());
    let menu = MenuTrigger(trigger).dropdown_menu(move |menu, _, _| {
        if deleted {
            return menu.item(PopupMenuItem::new(tr("moment.message.deleted")).disabled(true));
        }
        let copy = copy.clone();
        let menu =
            menu.item(PopupMenuItem::new(tr("message_ui.copy")).disabled(!copy_enabled).on_click(
                move |_, window, cx| copy.emit(UiEventKind::CopyMessagePressed, window, cx),
            ));
        let commands = [
            (Mutation::Reply, "message_ui.reply"),
            (Mutation::React, "message_ui.react"),
            (Mutation::DeleteLocal, "message_ui.delete_local"),
        ];
        let mut menu = commands.into_iter().fold(menu, |menu, (command, label)| {
            menu.item(PopupMenuItem::new(tr(label)).disabled(command.check().is_err()))
        });
        if own {
            for (command, label) in [
                (Mutation::Edit, "message_ui.edit"),
                (Mutation::DeleteEveryone, "message_ui.delete_everyone"),
            ] {
                menu = menu.item(PopupMenuItem::new(tr(label)).disabled(command.check().is_err()));
            }
        }
        menu.separator().item(PopupMenuItem::new(tr("message_ui.unavailable")).disabled(true))
    });
    let menu = menu.on_open_change(move |open, window, cx| {
        if !open && let Some(focus) = menu_focus.as_ref() {
            window.focus(focus, cx);
        }
    });
    let focused = state.focus.as_ref().is_some_and(|f| f.is_focused(window));
    let menu_shown = focused
        || state.highlighted
        || state.menu_focus.as_ref().is_some_and(|f| f.is_focused(window));
    let failed = row.outbound && matches!(payload.receipt, Receipt::Failed { .. });
    v_flex()
        .id(domain_id("moment", &row.moment.id))
        .group("moment-row")
        .role(Role::ListItem)
        .aria_label(super::accessible_label(row))
        .aria_selected(state.highlighted)
        .test_support()
        .when_some(state.focus.as_ref(), |col, focus| col.track_focus(focus))
        .w_full()
        .min_w_0()
        .gap_1()
        .when_some(row.day_header.clone(), |col, day| col.child(super::day_header(day, cx)))
        .child(
            h_flex()
                .w_full()
                .min_w_0()
                .items_start()
                .gap_2()
                .when(row.outbound, |line| line.justify_end())
                .when(!row.outbound, |line| {
                    line.child(
                        div()
                            .id("message-avatar")
                            .test_support()
                            .aria_label(ramflux_desktop_navigation::row::initial(&row.author))
                            .flex_none()
                            .w(rems(40.0 / 16.0))
                            .when(!state.joined, |avatar| {
                                avatar.child(catalog::avatar(
                                    ramflux_desktop_navigation::row::initial(&row.author),
                                    if agent {
                                        catalog::AvatarKind::Agent
                                    } else {
                                        catalog::AvatarKind::Person
                                    },
                                    theme,
                                ))
                            }),
                    )
                })
                .child(
                    v_flex()
                        .id("message-bubble")
                        .test_support()
                        .min_w_0()
                        .max_w(rems(theme.bubble_max / 16.0))
                        .when_some(body_width, |bubble, width| bubble.w(width))
                        .when(state.highlighted, |col| {
                            col.border_2()
                                .border_color(cx.theme().ring)
                                .rounded(rems(theme.radius_bubble / 16.0))
                        })
                        .when(focused, |col| col.focus_ring_style(window, cx))
                        .when(failed, |col| {
                            col.rounded(rems(theme.radius_bubble / 15.0))
                                .border_1()
                                .border_color(theme.danger)
                        })
                        .child(shell)
                        .children(receipt_line(payload, row, sink, cx)),
                )
                // Design 02: the message actions appear on hover or focus.
                .child(
                    div()
                        .when(!menu_shown, |slot| {
                            slot.opacity(0.).group_hover("moment-row", |style| style.opacity(1.))
                        })
                        .child(menu),
                ),
        )
        .into_any_element()
}

/// The receipt beside the time of an own message (design `B23`): a clock
/// while sending, one check when sent, a double check when delivered or read.
/// A failure or a wait for the node is told under the bubble instead.
fn receipt_meta(payload: &MessagePayload, outbound: bool) -> Option<impl IntoElement> {
    use gpui_kit::assets::IconName as Catalog;
    let icon = match payload.receipt {
        _ if !outbound => return None,
        Receipt::Failed { .. } | Receipt::WaitingForNode { .. } => return None,
        Receipt::Sending => Catalog::Clock,
        Receipt::Sent => Catalog::Check,
        Receipt::Delivered | Receipt::Read => Catalog::CheckCheck,
    };
    Some(
        div()
            .id("message-receipt")
            .aria_label(i18n::receipt(payload.receipt))
            .test_support()
            .child(Icon::new(icon).xsmall()),
    )
}

/// The line under a bubble whose send failed or waits for the node: the
/// state in words, and Retry when the failure can be retried.
fn receipt_line(
    payload: &MessagePayload,
    row: &MomentRow,
    sink: &EventSink,
    cx: &App,
) -> Option<gpui_kit::AnyElement> {
    let theme = palette(cx);
    let failed = matches!(payload.receipt, Receipt::Failed { .. });
    let waiting = matches!(payload.receipt, Receipt::WaitingForNode { .. });
    if !(failed || waiting) {
        return None;
    }
    let retry = sink.clone();
    let color = if failed { crate::tone::text(theme.danger, cx) } else { theme.secondary };
    Some(
        h_flex()
            .id("message-receipt")
            .aria_label(i18n::receipt(payload.receipt))
            .test_support()
            .pt_1()
            .gap_1()
            .text_size(rems(theme.size_meta / 15.0))
            .text_color(color)
            .when(failed, |line| line.child(Icon::new(IconName::CircleAlert).xsmall()))
            .child(i18n::receipt(payload.receipt))
            .when_some(row.refs.send_retry_ms, |line, ms| {
                line.child("·").child(i18n::tr_args(
                    "moment.message.retry_in",
                    &[("seconds", &ms.div_ceil(1000).to_string())],
                ))
            })
            .when(payload.receipt == (Receipt::Failed { retryable: true }), |line| {
                line.child(
                    gpui_kit::component::button::Button::new("message-retry")
                        .ghost()
                        .xsmall()
                        .label(tr("moment.message.retry"))
                        .on_click(move |_, window, cx| {
                            retry.emit(UiEventKind::RetryMessagePressed, window, cx)
                        }),
                )
            })
            .into_any_element(),
    )
}
