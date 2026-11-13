// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Native row composition around the generated, presentation-only message shell.

use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _, ThemeStyled as _, h_flex,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, IntoElement, Role, TestSupportExt as _, Window, div, rems};
use ramflux_desktop_core::projection::messages::Mutation;
use ramflux_desktop_core::{MomentKind, ParticipantKind, UiEventKind};
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
        format!("{} · {}", row.author, tr("message_ui.agent")).into()
    } else {
        row.author.clone()
    };
    let (lifecycle, tone) = super::state_label(row);
    let show_author = !state.joined || row.author_warning.is_some();
    let content = v_flex()
        .min_w_0()
        .gap_1()
        .when(show_author && !row.outbound, |col| {
            col.child(
                div().id("message-sender").test_support().text_sm().font_semibold().child(sender),
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
        .child(body)
        .child(
            h_flex()
                .justify_end()
                .text_xs()
                .text_color(if row.outbound { theme.outgoing_meta } else { theme.incoming_meta })
                .child(i18n::time_of_day(row.moment.at)),
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
    v_flex()
        .id(domain_id("moment", &row.moment.id))
        .role(Role::ListItem)
        .aria_label(super::accessible_label(row))
        .aria_selected(state.highlighted)
        .test_support()
        .when_some(state.focus.as_ref(), |col, focus| col.track_focus(focus))
        .w_full()
        .min_w_0()
        .px_3()
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
                    line.child(div().flex_none().w(rems(40.0 / 16.0)).when(
                        !state.joined,
                        |avatar| {
                            avatar.child(catalog::avatar(
                                row.author
                                    .chars()
                                    .next()
                                    .map_or_else(|| "?".to_owned(), |c| c.to_string())
                                    .into(),
                                if agent {
                                    catalog::AvatarKind::Agent
                                } else {
                                    catalog::AvatarKind::Person
                                },
                                theme,
                            ))
                        },
                    ))
                })
                .child(
                    v_flex()
                        .id("message-bubble")
                        .test_support()
                        .min_w_0()
                        .max_w(rems(theme.bubble_max / 16.0))
                        .when(state.highlighted, |col| {
                            col.border_2()
                                .border_color(cx.theme().ring)
                                .rounded(rems(theme.radius_bubble / 16.0))
                        })
                        .when(focused, |col| col.focus_ring_style(window, cx))
                        .child(shell),
                )
                .child(menu),
        )
        .into_any_element()
}
