// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! A new group, before it exists: its name and first members.
//!
//! The form opens in the conversation field (from the context stack's "+"
//! or the command palette) and chooses the group conversation's id once (a
//! fresh random id from the SDK), so a retry after a failure names the same
//! conversation. Members are chosen from the accepted contacts; a contact
//! that cannot be added now (not reachable yet) is shown disabled with the
//! reason. Create runs the
//! policy's own check (`check_create_group`) first and explains a refusal
//! in place; then it emits [`UiEventKind::CreateGroupRequested`], which the
//! root turns into the `CreateGroup` intent. The form follows the outcome
//! in the store: the group conversation appearing once the whole creation
//! (the group, then every member add) has finished ends it
//! ([`NewGroupEvent::Created`]); a failure is shown with the daemon's reason
//! and keeps the name and members. Cancel ends it without creating
//! anything ([`NewGroupEvent::Cancelled`]).
//!
//! The name is the one this account keeps for the group; other members name
//! it themselves, which the form says. The form is view-local.

use crate::controls::DesignCheckbox;
use crate::style::DesignButton as _;
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    Context, Entity, EventEmitter, FocusHandle, IntoElement, Render, Role, SharedString,
    Subscription, Window, div,
};
use ramflux_desktop_core::{
    ContactLink, ContextId, GroupProblem, IntentError, Participant, ParticipantId, Scope, Store,
    UiEventKind, check_create_group, check_start_conversation, methods, selectors,
};
use ramflux_desktop_navigation::direction::h_flex;
use ramflux_desktop_navigation::scale::{space, text_caption, text_secondary};

use crate::app::{StoreEvent, StoreHandle};
use crate::bubble::EventSink;
use crate::i18n::{self, tr, tr_args};
use crate::ids::domain_id;

/// Scope name of the form's sink.
pub const SCOPE: &str = "new_group";

/// Where the form is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    /// Choosing a name and members.
    Editing,
    /// Create was pressed; `since` and `created` are the store's failure
    /// and finished-creation counts then.
    Creating {
        /// Failures recorded before the submission.
        since: u64,
        /// Group creations finished before the submission.
        created: u64,
    },
    /// The creation failed or was refused (translated reason).
    Failed(SharedString),
}

/// Reported to the workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NewGroupEvent {
    /// The group conversation exists now; show it.
    Created(ContextId),
    /// The form was closed without creating a group.
    Cancelled,
}

/// The form and its view-local state.
pub struct NewGroup {
    store: Entity<StoreHandle>,
    sink: Option<EventSink>,
    name: Entity<InputState>,
    context_id: Option<ContextId>,
    members: Vec<ParticipantId>,
    phase: Phase,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<NewGroupEvent> for NewGroup {}

impl NewGroup {
    /// A closed form following `store`.
    pub fn new(store: Entity<StoreHandle>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name = i18n::localized_input("group.new.name_placeholder", window, cx, |input| input);
        let subscriptions = vec![
            cx.subscribe(&store, |this, _, event: &StoreEvent, cx| {
                if matches!(event, StoreEvent::Changed(_)) {
                    this.observe(cx);
                }
            }),
            cx.subscribe_in(&name, window, |this, _, event, window, cx| match event {
                InputEvent::PressEnter { .. } => this.create(window, cx),
                InputEvent::Change => {
                    if matches!(this.phase, Phase::Failed(_)) {
                        this.phase = Phase::Editing;
                    }
                    cx.notify();
                }
                _ => {}
            }),
        ];
        Self {
            store,
            sink: None,
            name,
            context_id: None,
            members: Vec::new(),
            phase: Phase::Editing,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// Opens an empty form under a fresh conversation id, emitting through
    /// `sink` (the root's), and focuses the name.
    pub fn open(&mut self, sink: &EventSink, window: &mut Window, cx: &mut Context<Self>) {
        self.sink = Some(sink.scoped(Scope::Panel(SCOPE.to_owned())));
        self.members.clear();
        self.name.update(cx, |name, cx| name.set_value("", window, cx));
        match ramflux_desktop_sdk::fresh_conversation_id() {
            Ok(id) => {
                self.context_id = Some(id);
                self.phase = Phase::Editing;
            }
            Err(error) => {
                self.context_id = None;
                self.phase = Phase::Failed(SharedString::from(error));
            }
        }
        self.name.update(cx, |name, cx| name.focus(window, cx));
        cx.notify();
    }

    /// Closes the form.
    pub fn close(&mut self, cx: &mut Context<Self>) {
        if self.sink.take().is_some() {
            self.context_id = None;
            cx.notify();
        }
    }

    /// Whether the form is open.
    pub fn is_open(&self) -> bool {
        self.sink.is_some()
    }

    /// The id the group conversation gets.
    pub fn context_id(&self) -> Option<&ContextId> {
        self.context_id.as_ref()
    }

    /// The chosen members, in the order chosen.
    pub fn members(&self) -> &[ParticipantId] {
        &self.members
    }

    /// Progress.
    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    /// Chooses or unchooses a member.
    pub fn set_member(&mut self, member: ParticipantId, chosen: bool, cx: &mut Context<Self>) {
        self.members.retain(|m| m != &member);
        if chosen {
            self.members.push(member);
        }
        if matches!(self.phase, Phase::Failed(_)) {
            self.phase = Phase::Editing;
        }
        cx.notify();
    }

    /// Create pressed: checks, then emits the request.
    pub fn create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.phase, Phase::Creating { .. }) {
            return;
        }
        let (Some(context_id), Some(sink)) = (self.context_id.clone(), self.sink.clone()) else {
            return;
        };
        let title = self.name.read(cx).value().to_string();
        let (checked, since, created) = {
            let store = self.store.read(cx).store();
            let checked = check_create_group(store, &title, &self.members)
                .map(|(title, members)| (title.to_owned(), members))
                .map_err(|error| refusal(store, &error));
            (checked, store.failures_total(), store.group_creations())
        };
        match checked {
            Ok((title, members)) => {
                self.phase = Phase::Creating { since, created };
                sink.emit(
                    UiEventKind::CreateGroupRequested { context_id, title, members },
                    window,
                    cx,
                );
            }
            Err(reason) => self.phase = Phase::Failed(reason),
        }
        cx.notify();
    }

    /// Cancel pressed.
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        self.close(cx);
        cx.emit(NewGroupEvent::Cancelled);
    }

    /// Follows the store: the group appeared, or its creation failed.
    fn observe(&mut self, cx: &mut Context<Self>) {
        let Phase::Creating { since, created } = self.phase else {
            if self.is_open() {
                // Who can be chosen may have changed.
                cx.notify();
            }
            return;
        };
        let store = self.store.read(cx).store();
        let failure = store
            .failures_since(since)
            .iter()
            .rev()
            .find(|f| f.method == methods::GROUP_CREATE)
            .map(|f| i18n::backend_text(&f.message));
        if let Some(reason) = failure {
            self.phase = Phase::Failed(tr_args("group.new.failed", &[("reason", &reason)]));
            cx.notify();
            return;
        }
        // The group conversation can be listed as soon as the group exists;
        // it is shown only once the creation, which adds every member after
        // the group, has finished, so a first message reaches all of them.
        if store.group_creations() == created {
            return;
        }
        if let Some(id) = self.context_id.clone().filter(|id| store.context(id).is_some()) {
            self.close(cx);
            self.phase = Phase::Editing;
            cx.emit(NewGroupEvent::Created(id));
        }
    }
}

/// The contact's name as shown.
fn name(store: &Store, participant: &ParticipantId) -> String {
    store.participant(participant).map_or_else(|| participant.short(), |p| p.display.clone())
}

/// Why the policy refuses, with what to do about it.
fn refusal(store: &Store, error: &IntentError) -> SharedString {
    let named = |key: &str, who: &ParticipantId| tr_args(key, &[("name", &name(store, who))]);
    match error {
        IntentError::InvalidGroup(GroupProblem::EmptyTitle) => tr("group.new.error.empty_name"),
        IntentError::InvalidGroup(GroupProblem::TitleTooLong) => tr("group.new.error.long_name"),
        IntentError::InvalidGroup(GroupProblem::TitleControl) => tr("group.new.error.bad_name"),
        IntentError::InvalidGroup(GroupProblem::NoMembers) => tr("group.new.error.no_members"),
        IntentError::InvalidGroup(GroupProblem::TooManyMembers) => tr("group.new.error.too_many"),
        IntentError::NotReachable(who) => named("group.new.error.unreachable", who),
        IntentError::ContactLinkState { participant, .. }
        | IntentError::UnknownParticipant(participant)
        | IntentError::SelfParticipant(participant) => {
            named("group.new.error.not_a_contact", participant)
        }
        other => SharedString::from(other.to_string()),
    }
}

/// Why `contact` cannot be a first member now, if it cannot.
fn ineligible(store: &Store, contact: &Participant) -> Option<SharedString> {
    check_start_conversation(store, &contact.id).err().map(|error| match error {
        IntentError::NotReachable(_) => tr("dock.contacts.unreachable"),
        _ => tr("group.new.not_available"),
    })
}

impl Render for NewGroup {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.is_open() {
            return div().id("new-group").into_any_element();
        }
        let store = self.store.read(cx).store();
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let danger = crate::tone::text(theme.danger, cx);
        let contacts: Vec<Participant> = selectors::contacts(store)
            .into_iter()
            .filter(|p| p.link_state() == ContactLink::Accepted)
            .cloned()
            .collect();
        let rows = contacts.iter().map(|contact| {
            let reason = ineligible(store, contact);
            let chosen = self.members.contains(&contact.id);
            let id = contact.id.clone();
            h_flex()
                .id(domain_id("new-group-row", &contact.id))
                .gap(space(2.))
                .py(space(1.))
                .child(
                    DesignCheckbox::new(domain_id("new-group-member", &contact.id))
                        .label(contact.display.clone())
                        .checked(chosen)
                        .disabled(reason.is_some() && !chosen)
                        .on_change(cx.listener(move |this, checked: &bool, _, cx| {
                            this.set_member(id.clone(), *checked, cx);
                        })),
                )
                .child(div().flex_1())
                .children(
                    reason
                        .or_else(|| {
                            (contact.identity != ramflux_desktop_core::IdentityState::Tofu)
                                .then(|| i18n::identity_state(contact.identity))
                        })
                        .map(|text| div().text_size(text_caption()).text_color(muted).child(text)),
                )
        });
        let creating = matches!(self.phase, Phase::Creating { .. });
        let status = match &self.phase {
            Phase::Creating { .. } => Some((tr("group.new.creating"), muted)),
            Phase::Failed(reason) => Some((reason.clone(), danger)),
            Phase::Editing => None,
        };
        let this = cx.weak_entity();
        let creating_or_waiting = creating || self.context_id.is_none();
        let create = Button::new("new-group-create")
            .design_primary(cx)
            .design_label(tr("group.new.create"))
            .design_disabled(creating_or_waiting, cx)
            .on_click(cx.listener(|this, _, window, cx| this.create(window, cx)));
        let cancel = Button::new("new-group-cancel")
            .design_secondary(cx)
            .design_label(tr("group.new.cancel"))
            .on_click(cx.listener(|this, _, _, cx| this.cancel(cx)));
        let form = v_flex()
            .id("new-group-form")
            .test_support()
            .gap(space(4.))
            .child(
                v_flex()
                    .gap(space(1.))
                    .child(div().text_size(text_secondary()).child(tr("group.new.name")))
                    .child(
                        Input::new(&self.name)
                            .id("new-group-name")
                            .aria_label(tr("group.new.name")),
                    )
                    .child(
                        div()
                            .text_size(text_caption())
                            .text_color(muted)
                            .child(tr("group.new.name_help")),
                    ),
            )
            .child(
                v_flex()
                    .gap(space(1.))
                    .child(div().text_size(text_secondary()).child(tr("group.new.members")))
                    .when(contacts.is_empty(), |col| {
                        col.child(
                            div()
                                .id("new-group-no-contacts")
                                .test_support()
                                .text_size(text_caption())
                                .text_color(muted)
                                .child(tr("group.new.no_contacts")),
                        )
                    })
                    .children(rows),
            )
            .children(status.map(|(text, color)| {
                div()
                    .id("new-group-status")
                    .role(Role::Status)
                    .aria_label(text.clone())
                    .test_support()
                    .text_size(text_secondary())
                    .text_color(color)
                    .child(text)
            }));
        crate::modal::Modal::new("new-group", tr("group.new.title"), &self.focus, move |_, cx| {
            this.update(cx, |form, cx| form.cancel(cx)).ok();
        })
        .child(form)
        .button(cancel)
        .button(create)
        .render(cx)
    }
}
