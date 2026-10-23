// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Interactive blocks: form header, text input, select, action group and
//! approval.
//!
//! Inputs and selections are view-local until an action is invoked; the
//! moment level then adds the collected values to the `ActionInvoked` event
//! (rule B3). An approval block decides the grant it names through the same
//! policy as an intervention (high risk opens the hold strip).

use gpui_kit::component::StyledExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Selectable as _, Sizable as _, h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, IntoElement, SharedString, div};
use ramflux_desktop_core::{GrantId, GrantState, JsonValue, SurfaceAction, UiEventKind};

use super::{ActionRow, BlockContext};
use crate::i18n::{self, tr, tr_args};
use crate::ids::domain_id;

fn str_field<'a>(data: &'a JsonValue, key: &str) -> Option<&'a str> {
    data.get(key).and_then(JsonValue::as_str)
}

/// A form header: title and an optional explanation.
pub fn form(data: &JsonValue, cx: &App) -> Option<AnyElement> {
    let title = str_field(data, "title")?.to_owned();
    let text = str_field(data, "text").map(str::to_owned);
    Some(
        v_flex()
            .gap_1()
            .child(div().text_sm().font_semibold().child(title))
            .when_some(text, |col, text| {
                col.child(div().text_xs().text_color(cx.theme().muted_foreground).child(text))
            })
            .into_any_element(),
    )
}

/// A labeled single-line text input. Its state is view-local.
pub fn input(data: &JsonValue, context: &BlockContext<'_>, cx: &App) -> Option<AnyElement> {
    let label = str_field(data, "label")?.to_owned();
    let name = str_field(data, "name")?;
    let input = context.input?;
    Some(
        v_flex()
            .gap_1()
            .child(div().text_xs().text_color(cx.theme().muted_foreground).child(label.clone()))
            .child(Input::new(input).id(domain_id("field", &name)).aria_label(label).small())
            .into_any_element(),
    )
}

/// `options` as `(value, label)` pairs; plain strings use the value as label.
fn options(data: &JsonValue) -> Option<Vec<(String, String)>> {
    data.get("options")?
        .as_array()?
        .iter()
        .map(|option| match option {
            JsonValue::String(value) => Some((value.clone(), value.clone())),
            JsonValue::Array(pair) => {
                let value = pair.first()?.as_str()?.to_owned();
                let label = pair.get(1).and_then(JsonValue::as_str).unwrap_or(&value).to_owned();
                Some((value, label))
            }
            _ => None,
        })
        .collect()
}

/// A single choice among a few options, shown as a segmented row. The
/// selection is view-local (`FieldChanged` is consumed at the moment level).
pub fn select(data: &JsonValue, context: &BlockContext<'_>, cx: &App) -> Option<AnyElement> {
    let label = str_field(data, "label")?.to_owned();
    str_field(data, "name")?;
    let options = options(data)?;
    let selected = context
        .field
        .cloned()
        .or_else(|| str_field(data, "value").map(str::to_owned))
        .unwrap_or_default();
    let buttons = options.into_iter().map(|(value, text)| {
        let sink = context.sink.clone();
        let is_selected = value == selected;
        Button::new(domain_id("option", &value))
            .small()
            .outline()
            .selected(is_selected)
            .label(text)
            .on_click(move |_, window, cx| {
                sink.emit(UiEventKind::FieldChanged { value: value.clone() }, window, cx);
            })
    });
    Some(
        v_flex()
            .gap_1()
            .child(div().text_xs().text_color(cx.theme().muted_foreground).child(label))
            .child(h_flex().flex_wrap().gap_1().children(buttons))
            .into_any_element(),
    )
}

/// The listed surface actions as buttons.
pub fn action_group(ids: &[&str], context: &BlockContext<'_>, cx: &App) -> Option<AnyElement> {
    let actions: Vec<SurfaceAction> =
        ids.iter().filter_map(|id| context.actions.iter().find(|a| a.id == *id).cloned()).collect();
    if actions.is_empty() {
        return None;
    }
    let row = ActionRow { hash: context.hash, blocked: context.blocked };
    Some(row.render("block-actions", &actions, &context.sink, cx))
}

/// An approval bound to a grant: summary, risk, scope and Approve / Deny
/// while pending, the decision afterwards.
pub fn approval(data: &JsonValue, context: &BlockContext<'_>, cx: &App) -> Option<AnyElement> {
    let grant_id = GrantId::new(str_field(data, "grant_id")?);
    let summary = str_field(data, "summary")?.to_owned();
    let grant = context.grants.get(&grant_id)?;
    let risk_color =
        if grant.risk.requires_confirmation() { cx.theme().danger } else { cx.theme().warning };
    let decision: AnyElement = if let Some(failure) = grant.failure {
        crate::moments::grant_failure::render(failure, cx)
    } else if grant.state == GrantState::Pending {
        let approve_sink = context.sink.clone();
        let deny_sink = context.sink.clone();
        let approve_label: SharedString = tr(if grant.risk.requires_confirmation() {
            "moment.intervention.approve_hold"
        } else {
            "moment.intervention.approve"
        });
        h_flex()
            .gap_2()
            .child(Button::new("approve").primary().small().label(approve_label).on_click(
                move |_, window, cx| approve_sink.emit(UiEventKind::ApprovePressed, window, cx),
            ))
            .child(
                Button::new("deny")
                    .outline()
                    .small()
                    .label(tr("moment.intervention.deny"))
                    .on_click(move |_, window, cx| {
                        deny_sink.emit(UiEventKind::DenyPressed, window, cx)
                    }),
            )
            .into_any_element()
    } else {
        div()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(i18n::grant_state(grant.state))
            .into_any_element()
    };
    Some(
        v_flex()
            .gap_2()
            .p_2()
            .border_1()
            .border_color(if grant.state == GrantState::Pending {
                risk_color
            } else {
                cx.theme().border
            })
            .rounded(cx.theme().radius)
            .child(
                h_flex()
                    .gap_2()
                    .items_start()
                    .child(Icon::new(IconName::TriangleAlert).small().text_color(risk_color))
                    .child(
                        v_flex().min_w_0().gap_1().child(div().text_sm().child(summary)).child(
                            h_flex()
                                .gap_2()
                                .text_xs()
                                .child(div().text_color(risk_color).child(tr_args(
                                    "moment.intervention.risk",
                                    &[("risk", &i18n::risk(grant.risk))],
                                )))
                                .child(
                                    div()
                                        .font_family(cx.theme().mono_font_family.clone())
                                        .text_color(cx.theme().muted_foreground)
                                        .child(grant.scope.clone()),
                                ),
                        ),
                    ),
            )
            .child(decision)
            .into_any_element(),
    )
}
