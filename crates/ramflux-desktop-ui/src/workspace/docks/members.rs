// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The members dock: members of the current group or space with their roles,
//! their kind as far as it is proven, a warning on a name that could pass
//! for another's (see [`crate::identity`]), their safety, and the
//! governance actions. Role changes, kicks and bans are
//! high risk: the root opens the hold-to-confirm strip before anything is
//! sent. Adding a contact and creating an invitation are low risk, but they
//! go through the same identity gate as a new group: a contact whose key
//! changed or forked is refused (the buttons are disabled with the reason),
//! and one whose verification went stale first asks, inline, to verify
//! again or continue; nothing is requested before Continue.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::StyledExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{ActiveTheme as _, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Context, Entity, IntoElement, Render, Subscription, Window, div};
use ramflux_desktop_core::{
    ContextId, ContextKind, GovernanceOp, Participant, Role, Scope, UiEventKind, selectors,
};

use super::{empty_line, section};
use crate::app::{StoreEvent, StoreHandle};
use crate::bubble::EventSink;
use crate::i18n::{self, tr, tr_args};
use crate::ids::domain_id;

/// The members dock content.
pub struct MembersDock {
    store: Entity<StoreHandle>,
    sink: EventSink,
    context_id: Option<ContextId>,
    /// The add or invite waiting for the user to acknowledge a stale
    /// verification of its contact.
    asking: Option<GovernanceOp>,
    _subscription: Subscription,
}

impl MembersDock {
    /// A dock over `store`; `sink` is the root sink (the dock adds its scopes).
    pub fn new(store: Entity<StoreHandle>, sink: EventSink, cx: &mut Context<Self>) -> Self {
        let subscription = cx.subscribe(&store, |this, _, event: &StoreEvent, cx| {
            if let StoreEvent::Changed(changes) = event
                && changes.touches_docks()
            {
                this.forget_settled_question(cx);
                cx.notify();
            }
        });
        Self { store, sink, context_id: None, asking: None, _subscription: subscription }
    }

    /// Drops the pending question once its contact is no longer stale
    /// (verified again, or the key changed and the contact is refused).
    fn forget_settled_question(&mut self, cx: &Context<Self>) {
        let store = self.store.read(cx).store();
        let stale = self
            .asking
            .as_ref()
            .and_then(GovernanceOp::target)
            .and_then(|id| store.participant(id))
            .is_some_and(|p| p.safety.is_stale());
        if !stale {
            self.asking = None;
        }
    }

    /// Shows the members of `context_id`.
    pub fn set_context(&mut self, context_id: Option<ContextId>, cx: &mut Context<Self>) {
        if self.context_id != context_id {
            self.context_id = context_id;
            self.asking = None;
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
            .accessibility_label(tr_args("dock.members.actions_for", &[("name", &member.display)]))
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
    v_flex()
        .id(domain_id("member", &member.id))
        .test_support()
        .px_3()
        .py_1()
        .gap_1()
        .child(
            h_flex()
                .gap_2()
                .child(
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_2()
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_sm()
                                .font_medium()
                                .child(member.display.clone()),
                        )
                        .when(member.named(), |row| {
                            row.child(
                                div()
                                    .flex_none()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(member.id.short()),
                            )
                        }),
                )
                // Not on this account's own row: it says nothing to the user.
                .when(!is_self, |row| {
                    row.children(crate::identity::kind_mark("member-kind", member.kind_claim, cx))
                })
                .child(
                    div().text_xs().text_color(cx.theme().muted_foreground).child(i18n::role(role)),
                )
                .when(member.safety.needs_attention() && !member.safety.is_stale(), |row| {
                    row.child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().danger)
                            .child(i18n::safety(member.safety)),
                    )
                })
                .children(crate::identity::stale_note("member-stale", member.safety, cx))
                .children(actions),
        )
        .children(crate::identity::name_warning("member-name-warning", member, cx))
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
        let candidates: Vec<Participant> = selectors::contacts(store)
            .into_iter()
            .filter(|p| !context.participants.contains(&p.id))
            .cloned()
            .collect();
        v_flex()
            .id("dock-members")
            .size_full()
            .overflow_y_scroll()
            .child(section("dock.members.list", cx))
            .children(members.iter().map(|(member, role)| {
                member_row(member, *role, me.as_ref() == Some(&member.id), &sink, cx)
            }))
            .child(section("dock.members.grow", cx))
            .children(candidates.into_iter().map(|candidate| {
                let ops = [
                    GovernanceOp::AddMember {
                        participant_id: candidate.id.clone(),
                        role: Role::Member,
                    },
                    GovernanceOp::CreateInvite {
                        participant_id: candidate.id.clone(),
                        role: Role::Member,
                    },
                ];
                let [add, invite] = ops;
                candidate_row(
                    &candidate,
                    [(add, add_reason.clone()), (invite, invite_reason.clone())],
                    self.asking.as_ref(),
                    &sink,
                    cx,
                )
            }))
    }
}

/// How a button acts on a candidate: straight away, or by asking first.
fn on_press(
    safety: ramflux_desktop_core::SafetyState,
    op: GovernanceOp,
    sink: &EventSink,
    cx: &Context<MembersDock>,
) -> impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static {
    let direct = op_sink(sink, op.clone());
    let ask = cx.listener(move |this, _, _, cx| {
        this.asking = Some(op.clone());
        cx.notify();
    });
    move |event, window, cx| {
        if safety.is_stale() {
            ask(event, window, cx);
        } else {
            direct(event, window, cx);
        }
    }
}

/// One contact that could join: Add and Invite. A key change or fork
/// disables both with the reason; a stale verification asks first.
fn candidate_row(
    candidate: &Participant,
    [(add, add_reason), (invite, invite_reason)]: [(GovernanceOp, Option<gpui_kit::SharedString>);
        2],
    asking: Option<&GovernanceOp>,
    sink: &EventSink,
    cx: &Context<MembersDock>,
) -> AnyElement {
    let safety = candidate.safety;
    let refused = safety.refuses_contact().then(|| i18n::safety(safety));
    let reason = |backend: Option<gpui_kit::SharedString>| refused.clone().or(backend);
    let question =
        asking.filter(|op| op.target() == Some(&candidate.id) && safety.is_stale()).map(|op| {
            let key = if matches!(op, GovernanceOp::CreateInvite { .. }) {
                "identity.ack.invite"
            } else {
                "identity.ack.add"
            };
            (op.clone(), tr_args(key, &[("name", &candidate.display)]))
        });
    let verify = {
        let (sink, id) = (sink.clone(), candidate.id.clone());
        move |_: &gpui_kit::ClickEvent, window: &mut Window, cx: &mut App| {
            sink.emit(UiEventKind::VerifySafetyPressed { participant_id: id.clone() }, window, cx);
        }
    };
    v_flex()
        .id(domain_id("member-add", &candidate.id))
        .test_support()
        .gap_1()
        .px_3()
        .py_1()
        .child(
            h_flex()
                .gap_2()
                .child(div().flex_1().text_sm().child(candidate.display.clone()))
                .children(
                    refused
                        .clone()
                        .map(|text| div().text_xs().text_color(cx.theme().danger).child(text)),
                )
                .children(crate::identity::stale_note("member-add-stale", safety, cx))
                .child(crate::gate::gated(
                    Button::new("add")
                        .xsmall()
                        .outline()
                        .label(tr("dock.members.add"))
                        .on_click(on_press(safety, add, sink, cx)),
                    reason(add_reason),
                ))
                .child(crate::gate::gated(
                    Button::new("invite")
                        .xsmall()
                        .ghost()
                        .label(tr("dock.members.invite"))
                        .on_click(on_press(safety, invite, sink, cx)),
                    reason(invite_reason),
                )),
        )
        .children(question.map(|(op, prompt)| {
            let proceed = {
                let direct = op_sink(sink, op);
                cx.listener(move |this, event, window, cx| {
                    this.asking = None;
                    direct(event, window, cx);
                    cx.notify();
                })
            };
            crate::identity::stale_ack(
                "member-add-stale-ack",
                prompt,
                Box::new(verify),
                Box::new(proceed),
                cx,
            )
        }))
        .into_any_element()
}
