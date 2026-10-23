// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The members dock: members of the current group or space with their roles
//! and safety, and the governance actions. Role changes, kicks and bans are
//! high risk: the root opens the hold-to-confirm strip before anything is
//! sent. Adding a contact and creating an invitation are low risk.

use gpui_kit::component::StyledExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{ActiveTheme as _, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Context, Entity, IntoElement, Render, Subscription, Window, div};
use ramflux_desktop_core::{
    ContextId, ContextKind, GovernanceOp, Participant, ParticipantId, Role, Scope, UiEventKind,
    selectors,
};

use super::{empty_line, section};
use crate::app::{StoreEvent, StoreHandle};
use crate::bubble::EventSink;
use crate::i18n::{self, tr};
use crate::ids::domain_id;

/// The members dock content.
pub struct MembersDock {
    store: Entity<StoreHandle>,
    sink: EventSink,
    context_id: Option<ContextId>,
    _subscription: Subscription,
}

impl MembersDock {
    /// A dock over `store`; `sink` is the root sink (the dock adds its scopes).
    pub fn new(store: Entity<StoreHandle>, sink: EventSink, cx: &mut Context<Self>) -> Self {
        let subscription = cx.subscribe(&store, |_, _, event: &StoreEvent, cx| {
            if let StoreEvent::Changed(changes) = event
                && changes.touches_docks()
            {
                cx.notify();
            }
        });
        Self { store, sink, context_id: None, _subscription: subscription }
    }

    /// Shows the members of `context_id`.
    pub fn set_context(&mut self, context_id: Option<ContextId>, cx: &mut Context<Self>) {
        if self.context_id != context_id {
            self.context_id = context_id;
            cx.notify();
        }
    }
}

fn op_sink(
    sink: &EventSink,
    op: GovernanceOp,
) -> impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static {
    let sink = sink.clone();
    move |_, window, cx| sink.emit(UiEventKind::GovernanceRequested { op: op.clone() }, window, cx)
}

fn member_row(
    member: &Participant,
    role: Role,
    is_self: bool,
    sink: &EventSink,
    cx: &App,
) -> AnyElement {
    let id = member.id.clone();
    let menu_sink = sink.clone();
    let actions = (!is_self && role != Role::Owner).then(|| {
        Button::new("member-actions")
            .ghost()
            .xsmall()
            .icon(IconName::Ellipsis)
            .tooltip(tr("dock.members.actions"))
            .dropdown_menu(move |menu, _, _| {
                let target = |op: GovernanceOp| {
                    PopupMenuItem::new(label(&op)).on_click(op_sink(&menu_sink, op))
                };
                let next_role = if role == Role::Admin { Role::Member } else { Role::Admin };
                menu.item(target(GovernanceOp::SetRole {
                    participant_id: id.clone(),
                    role: next_role,
                }))
                .separator()
                .item(target(GovernanceOp::Kick { participant_id: id.clone() }))
                .item(target(GovernanceOp::Ban { participant_id: id.clone() }))
            })
    });
    h_flex()
        .id(domain_id("member", &member.id))
        .gap_2()
        .px_3()
        .py_1()
        .child(div().flex_1().min_w_0().text_sm().font_medium().child(member.display.clone()))
        .child(div().text_xs().text_color(cx.theme().muted_foreground).child(i18n::role(role)))
        .when(member.safety.needs_attention(), |row| {
            row.child(
                div().text_xs().text_color(cx.theme().danger).child(i18n::safety(member.safety)),
            )
        })
        .children(actions)
        .into_any_element()
}

fn label(op: &GovernanceOp) -> gpui_kit::SharedString {
    match op {
        GovernanceOp::SetRole { role: Role::Admin, .. } => tr("dock.members.make_admin"),
        GovernanceOp::SetRole { .. } => tr("dock.members.make_member"),
        GovernanceOp::Kick { .. } => tr("dock.members.kick"),
        GovernanceOp::Ban { .. } => tr("dock.members.ban"),
        GovernanceOp::RemoveMember { .. } => tr("dock.members.remove"),
        GovernanceOp::AddMember { .. } => tr("dock.members.add"),
        GovernanceOp::CreateInvite { .. } => tr("dock.members.invite"),
    }
}

impl Render for MembersDock {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let store = self.store.read(cx).store();
        let add_reason =
            crate::gate::reason(store, ramflux_desktop_core::methods::GROUP_ADD_MEMBER);
        let invite_reason =
            crate::gate::reason(store, ramflux_desktop_core::methods::GROUP_CREATE_INVITE);
        let context = self.context_id.as_ref().and_then(|id| store.context(id)).cloned();
        let Some(context) = context.filter(|c| c.kind != ContextKind::Direct) else {
            return v_flex()
                .id("dock-members")
                .size_full()
                .child(empty_line("dock.members.not_group", cx));
        };
        let sink = self
            .sink
            .scoped(Scope::Context(context.id.clone()))
            .scoped(Scope::Panel(super::DockTool::Members.scope_name()));
        let me = selectors::self_participant(store);
        let members: Vec<(Participant, Role)> = selectors::participants(store, &context.id)
            .into_iter()
            .map(|p| (p.clone(), context.role_of(&p.id)))
            .collect();
        let candidates: Vec<(ParticipantId, String)> = selectors::contacts(store)
            .into_iter()
            .filter(|p| !context.participants.contains(&p.id))
            .map(|p| (p.id.clone(), p.display.clone()))
            .collect();
        let invite = op_sink(&sink, GovernanceOp::CreateInvite { role: Role::Member });
        v_flex()
            .id("dock-members")
            .size_full()
            .overflow_y_scroll()
            .child(section("dock.members.list", cx))
            .children(members.iter().map(|(member, role)| {
                member_row(member, *role, me.as_ref() == Some(&member.id), &sink, cx)
            }))
            .child(section("dock.members.grow", cx))
            .children(candidates.into_iter().map(|(id, name)| {
                let add = op_sink(
                    &sink,
                    GovernanceOp::AddMember { participant_id: id.clone(), role: Role::Member },
                );
                h_flex()
                    .id(domain_id("member-add", &id))
                    .gap_2()
                    .px_3()
                    .py_1()
                    .child(div().flex_1().text_sm().child(name))
                    .child(crate::gate::gated(
                        Button::new("add")
                            .xsmall()
                            .outline()
                            .label(tr("dock.members.add"))
                            .on_click(add),
                        add_reason.clone(),
                    ))
            }))
            .child(
                div().px_3().py_2().child(crate::gate::gated(
                    Button::new("members-invite")
                        .small()
                        .outline()
                        .icon(IconName::Plus)
                        .label(tr("dock.members.invite"))
                        .on_click(invite),
                    invite_reason,
                )),
            )
    }
}
