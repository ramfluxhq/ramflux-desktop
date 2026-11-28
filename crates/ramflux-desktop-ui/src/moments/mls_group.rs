// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Service lines of an MLS group timeline (design C19 style): membership,
//! settings, an undone change, a deleted message and the floor of what this
//! device can show. One centred, quiet line each; they carry no action.
//! Names are bidi-isolated, a missing name reads "Member" (never the
//! anchor), and the local account reads "You".

use crate::style::DesignButton as _;
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::Button;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{App, IntoElement, SharedString, div};
use ramflux_desktop_core::ContextId;
use ramflux_desktop_core::UiEventKind;
use ramflux_desktop_core::mls_group::{MlsName, MlsPanel, MlsServiceLine, UndoOffer};
use ramflux_desktop_navigation::direction::h_flex;
use ramflux_desktop_navigation::scale::{space, text_caption};

use crate::bubble::EventSink;
use crate::i18n::{tr, tr_args, tr_count};
use crate::workspace::mls_group::name_text;

fn name(who: &MlsName) -> SharedString {
    name_text(who.display.as_deref(), who.is_me)
}

fn opt(who: Option<&MlsName>) -> Option<SharedString> {
    who.map(name)
}

/// "A joined", or "A and 12 others joined".
fn many(single: &str, many: &str, who: &[MlsName]) -> SharedString {
    match who {
        [] => SharedString::default(),
        [one] => tr_args(single, &[("name", &name(one))]),
        [first, rest @ ..] => {
            tr_args(many, &[("name", &name(first)), ("count", &rest.len().to_string())])
        }
    }
}

/// The sentence of a service line.
pub fn sentence(line: &MlsServiceLine) -> SharedString {
    match line {
        MlsServiceLine::Joined { who } => {
            many("group.timeline.joined", "group.timeline.joined_many", who)
        }
        MlsServiceLine::Left { who } => {
            many("group.timeline.left", "group.timeline.left_many", who)
        }
        MlsServiceLine::Removed { by, target } => match opt(by.as_ref()) {
            Some(by) => tr_args("group.timeline.removed", &[("by", &by), ("name", &name(target))]),
            None => tr_args("group.timeline.removed_passive", &[("name", &name(target))]),
        },
        MlsServiceLine::Banned { by, target } => match opt(by.as_ref()) {
            Some(by) => tr_args("group.timeline.banned", &[("by", &by), ("name", &name(target))]),
            None => tr_args("group.timeline.banned_passive", &[("name", &name(target))]),
        },
        MlsServiceLine::RoleChanged { by, target, admin } => {
            let key =
                if *admin { "group.timeline.role_admin" } else { "group.timeline.role_member" };
            match opt(by.as_ref()) {
                Some(by) => tr_args(key, &[("by", &by), ("name", &name(target))]),
                // A role change with nobody named reads as a plain update.
                None => tr("group.timeline.updated"),
            }
        }
        MlsServiceLine::Renamed { by, name: new } => {
            let by = opt(by.as_ref()).unwrap_or_else(|| tr("group.member.unnamed"));
            tr_args(
                "group.timeline.name",
                &[("by", &by), ("name", &crate::i18n::bidi::isolate(new))],
            )
        }
        MlsServiceLine::Approval { by, on } => {
            let by = opt(by.as_ref()).unwrap_or_else(|| tr("group.member.unnamed"));
            let key =
                if *on { "group.timeline.approval_on" } else { "group.timeline.approval_off" };
            tr_args(key, &[("by", &by)])
        }
        MlsServiceLine::Void { by } => match opt(by.as_ref()) {
            Some(by) => tr_args("group.timeline.void_by", &[("by", &by)]),
            None => tr("group.timeline.void"),
        },
        MlsServiceLine::Undecryptable { count } => tr_count("group.timeline.undecryptable", *count),
        MlsServiceLine::Commit { line, .. } => sentence(line),
        MlsServiceLine::Updated => tr("group.timeline.updated"),
        MlsServiceLine::Deleted => tr("group.timeline.deleted"),
        MlsServiceLine::Floor => tr("group.timeline.floor"),
    }
}

fn icon(line: &MlsServiceLine) -> IconName {
    match line {
        MlsServiceLine::Joined { .. } | MlsServiceLine::Left { .. } => IconName::Users,
        MlsServiceLine::Removed { .. } | MlsServiceLine::Banned { .. } => IconName::UserX,
        MlsServiceLine::RoleChanged { .. } => IconName::Shield,
        MlsServiceLine::Renamed { .. } | MlsServiceLine::Approval { .. } => IconName::Settings,
        MlsServiceLine::Void { .. } => IconName::Undo2,
        MlsServiceLine::Undecryptable { .. } => IconName::LockKeyhole,
        MlsServiceLine::Commit { line, .. } => icon(line),
        MlsServiceLine::Deleted => IconName::Trash,
        MlsServiceLine::Updated => IconName::Info,
        MlsServiceLine::Floor => IconName::Clock,
    }
}

/// Renders a service line: a centred, quiet sentence with a small icon. The
/// line of a commit row also carries the "Undone" mark of a row a void row
/// undid, and the "Undo this change" entry of the head commit row (it opens
/// the confirmation; nothing is sent by the entry).
pub fn render(
    line: &MlsServiceLine,
    context: &ContextId,
    sink: &EventSink,
    cx: &App,
) -> impl IntoElement {
    let (inner, voided, undo) = match line {
        MlsServiceLine::Commit { line, voided, undo } => (line.as_ref(), *voided, *undo),
        other => (other, false, UndoOffer::No),
    };
    let text = sentence(inner);
    let group = ramflux_desktop_core::mls_group::group_of(context).unwrap_or_default().to_owned();
    let muted = cx.theme().muted_foreground;
    let action = match undo {
        UndoOffer::No => None,
        UndoOffer::Offer { log_seq } | UndoOffer::Busy { log_seq } => {
            let sink = sink.clone();
            let busy = matches!(undo, UndoOffer::Busy { .. });
            Some(
                Button::new("mls-undo-change")
                    .design_quiet(cx)
                    .design_label(tr("group.void.action"))
                    .design_disabled(busy, cx)
                    .on_click(move |_, window, cx| {
                        sink.emit(
                            UiEventKind::MlsPanelRequested {
                                panel: MlsPanel::ConfirmVoid { group_id: group.clone(), log_seq },
                            },
                            window,
                            cx,
                        );
                    })
                    .into_any_element(),
            )
        }
        UndoOffer::Undoing { .. } => Some(
            div()
                .id("mls-undo-undoing")
                .role(gpui_kit::Role::Status)
                .text_color(muted)
                .child(tr("group.void.undoing"))
                .into_any_element(),
        ),
    };
    h_flex()
        .id("mls-service-line")
        .aria_label(if voided {
            crate::i18n::clause(&text, &tr("group.void.undone"))
        } else {
            text.clone()
        })
        .w_full()
        .justify_center()
        .gap(space(2.))
        .text_size(text_caption())
        .text_color(muted)
        .child(Icon::new(icon(inner)).xsmall())
        .child(div().min_w_0().child(text))
        .children(
            voided.then(|| div().id("mls-undone-mark").flex_none().child(tr("group.void.undone"))),
        )
        .children(action)
        .test_support()
}
