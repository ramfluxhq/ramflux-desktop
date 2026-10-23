// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The developer panel (debug builds; toggle with secondary-shift-d).
//!
//! Scenario picker, timeline control (pause, step, seek), an agent proposal
//! for exercising the confirmation policy, store counts, and the event
//! inspector. Everything goes through [`StoreHandle`]; the panel never writes
//! the store itself.

use gpui_kit::component::StyledExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::description_list::DescriptionList;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Selectable as _, Sizable as _, h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, Entity, IntoElement, Render, SharedString, Subscription, WeakEntity, Window, div,
};
use ramflux_desktop_core::{GrantFilter, GrantId, IntentKind, ParticipantKind, Store, selectors};
use ramflux_desktop_test::{SCENARIO_NAMES, load_fixture};

use crate::app::{InspectorEntry, StoreEvent, StoreHandle, UiOutcome};
use crate::i18n::{self, tr, tr_args};
use crate::workspace::Workspace;

/// Seek step of the back and forward buttons.
const SEEK_STEP_MS: u64 = 1_000;
/// Number of inspector entries shown.
const SHOWN_ENTRIES: usize = 60;

/// The developer panel view.
pub struct DevPanel {
    store: Entity<StoreHandle>,
    workspace: WeakEntity<Workspace>,
    _subscription: Subscription,
}

impl DevPanel {
    /// A panel over `store`; agent proposals go to `workspace`.
    pub fn new(
        store: Entity<StoreHandle>,
        workspace: WeakEntity<Workspace>,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscription = cx.subscribe(&store, |_, _, _: &StoreEvent, cx| cx.notify());
        Self { store, workspace, _subscription: subscription }
    }

    /// Plays a bundled scenario from the start.
    pub fn load(&mut self, name: &str, cx: &mut Context<Self>) {
        if let Ok(scenario) = load_fixture(name) {
            self.store.update(cx, |store, cx| store.load_scenario(&scenario, cx));
        }
    }

    /// Seeks the active session.
    pub fn seek(&mut self, ms: u64, cx: &mut Context<Self>) {
        self.store.update(cx, |store, cx| store.seek(ms, cx));
    }

    fn pending_agent_grant(
        store: &Store,
    ) -> Option<(ramflux_desktop_core::ParticipantId, GrantId)> {
        let agent = store.participants().find(|p| p.kind == ParticipantKind::Agent)?.id.clone();
        let grant = selectors::grants(store, GrantFilter::Pending).first()?.id.clone();
        Some((agent, grant))
    }

    fn agent_approve(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((agent, grant_id)) = Self::pending_agent_grant(self.store.read(cx).store()) else {
            return;
        };
        let kind = IntentKind::Approve { grant_id, confirmed: true };
        self.workspace
            .update(cx, |workspace, cx| workspace.submit_agent_intent(agent, kind, window, cx))
            .ok();
    }

    fn section(title: SharedString, cx: &App) -> gpui_kit::Div {
        v_flex().gap_2().child(
            div().text_xs().font_semibold().text_color(cx.theme().muted_foreground).child(title),
        )
    }

    fn render_scenarios(&self, active: Option<&str>, cx: &mut Context<Self>) -> impl IntoElement {
        let buttons = SCENARIO_NAMES.into_iter().map(|name| {
            let short = name.split('_').next().unwrap_or(name).to_owned();
            Button::new(SharedString::from(format!("dev-scenario-{name}")))
                .xsmall()
                .outline()
                .selected(active == Some(name))
                .label(short)
                .tooltip(name)
                .on_click(cx.listener(move |this, _, _, cx| this.load(name, cx)))
        });
        Self::section(tr("dev.scenario"), cx).child(h_flex().flex_wrap().gap_1().children(buttons))
    }

    fn render_timeline(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let handle = self.store.read(cx);
        let (clock, next, paused) = handle
            .session()
            .map_or((0, None, false), |s| (s.clock_ms(), s.next_due(), s.is_paused()));
        let clock_line = tr_args("dev.clock", &[("time", &i18n::offset(clock))]);
        let next_line = next.map_or_else(
            || tr("dev.next_none"),
            |due| tr_args("dev.next", &[("time", &i18n::offset(due))]),
        );
        Self::section(tr("dev.timeline"), cx)
            .child(
                h_flex()
                    .gap_3()
                    .text_sm()
                    .font_family(cx.theme().mono_font_family.clone())
                    .child(div().id("dev-clock").child(clock_line))
                    .child(div().text_color(cx.theme().muted_foreground).child(next_line)),
            )
            .child(
                h_flex()
                    .flex_wrap()
                    .gap_1()
                    .child(
                        Button::new("dev-pause")
                            .xsmall()
                            .outline()
                            .icon(if paused { IconName::Play } else { IconName::Pause })
                            .label(tr(if paused { "dev.resume" } else { "dev.pause" }))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.store.update(cx, |store, cx| store.set_paused(!paused, cx));
                            })),
                    )
                    .child(
                        Button::new("dev-step")
                            .xsmall()
                            .outline()
                            .label(tr("dev.step"))
                            .disabled(next.is_none())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.store.update(cx, |store, cx| store.step(cx));
                            })),
                    )
                    .child(
                        Button::new("dev-restart")
                            .xsmall()
                            .outline()
                            .label(tr("dev.restart"))
                            .on_click(cx.listener(|this, _, _, cx| this.seek(0, cx))),
                    )
                    .child(
                        Button::new("dev-back").xsmall().outline().label(tr("dev.back")).on_click(
                            cx.listener(move |this, _, _, cx| {
                                this.seek(clock.saturating_sub(SEEK_STEP_MS), cx);
                            }),
                        ),
                    )
                    .child(
                        Button::new("dev-forward")
                            .xsmall()
                            .outline()
                            .label(tr("dev.forward"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.seek(clock.saturating_add(SEEK_STEP_MS), cx);
                            })),
                    ),
            )
    }

    fn render_agent(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let available = Self::pending_agent_grant(self.store.read(cx).store()).is_some();
        Self::section(tr("dev.agent"), cx).child(
            Button::new("dev-agent-approve")
                .xsmall()
                .outline()
                .label(tr("dev.agent_approve"))
                .disabled(!available)
                .on_click(cx.listener(|this, _, window, cx| this.agent_approve(window, cx))),
        )
    }

    fn render_counts(&self, cx: &App) -> impl IntoElement {
        let store = self.store.read(cx).store();
        let count = |n: usize| SharedString::from(n.to_string());
        let pending = selectors::grants(store, GrantFilter::Pending).len();
        Self::section(tr("dev.store"), cx).child(
            DescriptionList::new()
                .columns(2)
                .item(tr("dev.count.contexts"), count(store.contexts().count()), 1)
                .item(tr("dev.count.moments"), count(store.moments().count()), 1)
                .item(tr("dev.count.participants"), count(store.participants().count()), 1)
                .item(tr("dev.count.grants"), count(pending), 1)
                .item(tr("dev.count.objects"), count(store.objects().count()), 1)
                .item(tr("dev.count.tasks"), count(store.tasks().count()), 1)
                .item(tr("dev.count.seq"), SharedString::from(store.last_seq().to_string()), 1)
                .item(tr("dev.count.rejections"), count(store.rejections().len()), 1),
        )
    }

    fn render_inspector(&self, cx: &App) -> impl IntoElement {
        let handle = self.store.read(cx);
        let muted = cx.theme().muted_foreground;
        let danger = cx.theme().danger;
        let rows = handle.inspector().entries().rev().take(SHOWN_ENTRIES).map(|entry| {
            let (text, error) = describe(entry);
            div().text_color(if error { danger } else { muted }).child(text)
        });
        Self::section(tr("dev.inspector"), cx).child(
            v_flex()
                .id("dev-inspector")
                .gap_0p5()
                .text_xs()
                .font_family(cx.theme().mono_font_family.clone())
                .children(rows),
        )
    }
}

/// One-line technical description of an inspector entry, and whether it is an error.
fn describe(entry: &InspectorEntry) -> (String, bool) {
    match entry {
        InspectorEntry::Envelope { seq, at_ms, event, error } => match error {
            Some(error) => {
                (format!("#{seq} {} {event} rejected: {error}", i18n::offset(*at_ms)), true)
            }
            None => (format!("#{seq} {} {event}", i18n::offset(*at_ms)), false),
        },
        InspectorEntry::UiEvent { path, kind, outcome, elapsed } => {
            let result = match outcome {
                UiOutcome::Consumed { at } => format!("consumed at {at}"),
                UiOutcome::Intent { intent, result } => format!("{intent}: {result}"),
                UiOutcome::Ignored => "ignored".to_owned(),
            };
            (format!("ui {kind} [{path}] -> {result} ({}us)", elapsed.as_micros()), false)
        }
        InspectorEntry::Effect { label } => (format!("effect {label}"), false),
        InspectorEntry::Intent { intent, actor, result } => {
            (format!("intent {intent} by {actor}: {result}"), result.starts_with("refused"))
        }
    }
}

impl Render for DevPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.store.read(cx).session().map(|s| s.scenario_name().to_owned());
        let active = active.as_deref().and_then(|a| SCENARIO_NAMES.into_iter().find(|n| *n == a));
        v_flex()
            .id("dev-panel")
            .h_full()
            .w_96()
            .flex_shrink_0()
            .overflow_y_scroll()
            .gap_4()
            .p_3()
            .border_l_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().sidebar)
            .child(div().text_sm().font_semibold().child(tr("dev.title")))
            .child(self.render_scenarios(active, cx))
            .child(self.render_timeline(cx))
            .child(self.render_agent(cx))
            .child(self.render_counts(cx))
            .child(self.render_inspector(cx))
    }
}
