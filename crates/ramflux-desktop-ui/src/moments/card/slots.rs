// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Host-owned presentation: the approval card, the modal sheet and the link
//! confirmation.
//!
//! The approval and the link confirmation are drawn inside the host frame: a
//! heavy border and a filled heading strip with the shield glyph, which no
//! node of a tree can produce (no tree node draws that border, and no tree
//! Icon name maps to that glyph), so a card cannot imitate a host request.
//!
//! An approval draws its verified facts only from the host slot's `facts`,
//! which the SDK stamps from the verified approval row (requester, action,
//! scope, expiry, risk); never from tree text. Without facts the approval is
//! not awaiting this account's decision: it shows its outcome line and no
//! buttons. The card's explanation is shown as the author's words, labelled
//! as such; the author's task binding is not shown.
//!
//! A link sheet shows the host (on its own line), the exact URL in a
//! scrollable block, the author's name and a privacy note, with Cancel and a
//! single tap Open. Only a printable-ASCII https link can be opened; any
//! other link is refused as it is (never cleaned for display): the sheet
//! shows the refusal and Cancel only. The browser opens only after the
//! runtime answers the slot `opened` for the URL the sheet showed (the app
//! does that, not this module).

use gpui_kit::component::StyledExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, IntoElement, SharedString, Window, div, px};
use ramflux_desktop_core::{
    CardHostSlot, CardInput, CardNode, CardRecord, CardSlotOutcome, CardStatus, JsonValue,
    Timestamp, UiEventKind, https_host,
};
use ramflux_desktop_navigation::direction::h_flex;
use ramflux_desktop_navigation::scale::{space, text_caption, text_secondary, text_subheading};

use super::{CardSnapshot, View, attr_text, shown};
use crate::i18n::{tr, tr_args};
use crate::ids::domain_id;
use crate::style::DesignButton as _;

/// The host frame around a host request: a heavy accent border and a filled
/// heading strip with the shield glyph and `title`. Tree nodes never draw
/// this border or this glyph.
fn host_frame(title: SharedString, cx: &App) -> gpui_kit::Div {
    v_flex()
        .w_full()
        .rounded(cx.theme().radius)
        .border_2()
        .border_color(cx.theme().primary)
        .bg(cx.theme().background)
        .child(
            h_flex()
                .w_full()
                .gap(space(2.))
                .items_center()
                .px(space(3.))
                .py(space(1.5))
                .bg(cx.theme().primary)
                .text_color(cx.theme().primary_foreground)
                .child(Icon::new(gpui_kit::assets::IconName::ShieldCheck).small())
                .child(
                    div()
                        .text_size(text_caption())
                        .font_semibold()
                        .child(tr("moment.card.host_label")),
                )
                .child(div().text_size(text_secondary()).font_semibold().child(title)),
        )
}

/// The verified facts of an approval, as the SDK stamped them.
struct Facts {
    requester: String,
    action: Option<String>,
    scope: Vec<(String, String)>,
    expires_at: Option<i64>,
    risk: Option<String>,
}

impl Facts {
    /// The facts of an approval slot; `None` when the slot carries none (the
    /// approval is not awaiting this account's decision) or they do not read
    /// as the SDK writes them.
    fn of(slot: &CardHostSlot) -> Option<Self> {
        let facts = slot.request.get("facts")?.as_object()?;
        let text = |name: &str| facts.get(name).and_then(JsonValue::as_str).map(str::to_owned);
        let requester = text("requester").filter(|r| !r.is_empty())?;
        let scope: Vec<(String, String)> = facts
            .get("scope")?
            .as_array()?
            .iter()
            .filter_map(|row| {
                Some((
                    row.get("name")?.as_str()?.to_owned(),
                    row.get("value")?.as_str()?.to_owned(),
                ))
            })
            .collect();
        if scope.is_empty() {
            return None;
        }
        Some(Self {
            requester,
            action: text("action"),
            scope,
            expires_at: facts.get("expires_at").and_then(JsonValue::as_i64),
            risk: text("risk"),
        })
    }
}

/// The label of a verified scope row (fixed names only; others are not drawn).
fn scope_label(name: &str) -> Option<SharedString> {
    Some(match name {
        "conversation" => tr("moment.card.fact_conversation"),
        "task" => tr("moment.card.fact_task"),
        "step" => tr("moment.card.fact_step"),
        "tool_scope" => tr("moment.card.fact_tool_scope"),
        "grant" => tr("moment.card.fact_grant"),
        _ => return None,
    })
}

/// One fact line: a muted label and the value.
fn fact_row(label: SharedString, value: String, cx: &App) -> AnyElement {
    h_flex()
        .gap(space(2.))
        .items_start()
        .child(
            div()
                .min_w(px(96.))
                .text_size(text_caption())
                .text_color(cx.theme().muted_foreground)
                .child(label),
        )
        .child(div().text_size(text_caption()).child(shown(&value)))
        .into_any_element()
}

/// An approval card: the verified facts, the author's explanation and Deny /
/// Confirm while the runtime waits for this account's decision (then
/// "Waiting for confirmation" until the signed update). An approval without
/// verified facts shows its outcome line and no buttons.
pub fn approval(view: &View<'_>, node: &CardNode, window: &mut Window, cx: &mut App) -> AnyElement {
    let _ = window;
    let slot = view.tree.host_slots().iter().find(|s| {
        s.key == node.key && s.request.get("kind").and_then(|k| k.as_str()) == Some("approval")
    });
    let facts = slot.and_then(Facts::of);
    let explanation = attr_text(node, "explanation");
    let mut body = v_flex().w_full().gap(space(2.)).p(space(3.));
    if let Some(facts) = &facts {
        let mut rows = v_flex().gap(space(1.));
        rows = rows.child(fact_row(tr("moment.card.fact_requester"), facts.requester.clone(), cx));
        if let Some(action) = &facts.action {
            rows = rows.child(fact_row(tr("moment.card.fact_action"), action.clone(), cx));
        }
        for (name, value) in &facts.scope {
            if let Some(label) = scope_label(name) {
                rows = rows.child(fact_row(label, value.clone(), cx));
            }
        }
        if let Some(at) = facts.expires_at {
            let at = crate::i18n::format::date_time(Timestamp(at.saturating_mul(1000)));
            rows = rows.child(fact_row(tr("moment.card.fact_expires"), at, cx));
        }
        if let Some(risk) = &facts.risk {
            rows = rows.child(fact_row(tr("moment.card.fact_risk"), risk.clone(), cx));
        }
        body = body.child(rows);
    }
    if !explanation.is_empty() {
        body = body.child(
            v_flex()
                .gap(space(1.))
                .child(
                    div()
                        .text_size(text_caption())
                        .text_color(cx.theme().muted_foreground)
                        .child(tr("moment.card.author_words")),
                )
                .child(div().text_size(text_secondary()).child(explanation)),
        );
    }
    let outcome_line = |key: &'static str, cx: &App| {
        div()
            .text_size(text_caption())
            .text_color(cx.theme().muted_foreground)
            .child(tr(key))
            .into_any_element()
    };
    let decision: AnyElement = match (slot, &facts) {
        _ if pending_status(view.status) => outcome_line("moment.card.waiting_confirmation", cx),
        (Some(slot), Some(_)) if view.enabled(node) => {
            let deny_id = slot.id.clone();
            let confirm_id = slot.id.clone();
            let deny_view = view.sink.clone();
            let confirm_view = view.sink.clone();
            let deny_seen = view.seen_card();
            let confirm_seen = deny_seen.clone();
            h_flex()
                .gap(space(2.))
                .child(
                    Button::new(domain_id("card-deny", &node.key))
                        .small()
                        .design_secondary(cx)
                        .design_label(tr("moment.intervention.deny"))
                        .on_click(move |_, window, cx| {
                            deny_view.emit(
                                UiEventKind::CardSlotAnswered {
                                    slot_id: deny_id.clone(),
                                    outcome: CardSlotOutcome::Deny,
                                    seen: deny_seen.clone(),
                                },
                                window,
                                cx,
                            );
                        }),
                )
                .child(
                    Button::new(domain_id("card-confirm", &node.key))
                        .small()
                        .design_primary(cx)
                        .design_label(tr("moment.card.approval_confirm"))
                        .on_click(move |_, window, cx| {
                            confirm_view.emit(
                                UiEventKind::CardSlotAnswered {
                                    slot_id: confirm_id.clone(),
                                    outcome: CardSlotOutcome::Confirm,
                                    seen: confirm_seen.clone(),
                                },
                                window,
                                cx,
                            );
                        }),
                )
                .into_any_element()
        }
        (_, Some(_)) => div().into_any_element(),
        (_, None) => outcome_line("moment.card.approval_not_yours", cx),
    };
    host_frame(tr("moment.card.approval_title"), cx).child(body.child(decision)).into_any_element()
}

/// The modal sheet of a Modal node: its content in a dialog over a scrim,
/// closed by its close button (the runtime is told of the close).
pub fn modal_sheet(
    view: &View<'_>,
    node: &CardNode,
    content: &CardNode,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let close_local = view.local.clone();
    let close_view = view.sink.clone();
    let key = node.key.clone();
    let close_seen = view.seen(node);
    let author = shown(view.author.as_ref());
    let body = view.render(content, window, cx);
    div()
        .absolute()
        .inset_0()
        .bg(cx.theme().overlay)
        .flex()
        .items_center()
        .justify_center()
        .child(
            v_flex()
                .w(px(480.))
                .max_w_full()
                .gap(space(3.))
                .p(space(4.))
                .rounded(cx.theme().radius)
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().background)
                .child(
                    h_flex()
                        .justify_between()
                        .items_center()
                        .child(
                            div()
                                .text_size(text_subheading())
                                .font_semibold()
                                .child(tr_args("moment.card.modal_by", &[("author", &author)])),
                        )
                        .child(
                            Button::new(domain_id("card-modal-close", &node.key))
                                .small()
                                .design_quiet(cx)
                                .design_label(tr("moment.card.modal_close"))
                                .on_click(move |_, window, cx| {
                                    close_local.borrow_mut().modal = None;
                                    close_view.emit(
                                        UiEventKind::CardInteracted {
                                            key: key.clone(),
                                            event: CardInput::ModalClose,
                                            seen: close_seen.clone(),
                                        },
                                        window,
                                        cx,
                                    );
                                    window.refresh();
                                }),
                        ),
                )
                .child(body),
        )
        .into_any_element()
}

/// The link confirmation, when the runtime asked the host to show one: the
/// host on its own line, the full URL in a scrollable block, the author and
/// the privacy note, with Cancel and Open. A link the host refuses (not
/// https, or any character outside printable ASCII) is not shown at all:
/// only the refusal and Cancel are offered.
pub fn link_sheet(
    view: &View<'_>,
    record: &CardRecord,
    snap: &CardSnapshot,
    window: &mut Window,
    cx: &mut App,
) -> Option<AnyElement> {
    let _ = window;
    let link = record.link.as_ref()?;
    let slot_id = link.slot_id.clone();
    let cancel_id = slot_id.clone();
    let cancel_view = view.sink.clone();
    let open_view = view.sink.clone();
    let seen = view.seen_card();
    let cancel_seen = seen.clone();
    let author = shown(snap.author.as_ref());
    let host = https_host(&link.url).map(str::to_owned);
    let mut sheet = v_flex().w_full().gap(space(2.)).p(space(3.));
    match &host {
        Some(host) => {
            // The URL is printable ASCII (the host refuses anything else), so
            // it is drawn exactly as it opens.
            sheet = sheet
                .child(div().text_size(text_secondary()).font_semibold().child(host.clone()))
                .child(
                    div()
                        .id(domain_id("card-link-url", &slot_id))
                        .max_h(px(120.))
                        .overflow_y_scroll()
                        .text_size(text_secondary())
                        .font_family(cx.theme().mono_font_family.clone())
                        .child(link.url.clone()),
                );
        }
        None => {
            sheet = sheet.child(
                div()
                    .text_size(text_caption())
                    .text_color(cx.theme().danger)
                    .child(tr("moment.card.link_refused")),
            );
        }
    }
    sheet = sheet
        .child(
            div()
                .text_size(text_caption())
                .child(tr_args("moment.card.link_author", &[("author", &author)])),
        )
        .child(
            div()
                .text_size(text_caption())
                .text_color(cx.theme().muted_foreground)
                .child(tr("moment.card.link_privacy")),
        );
    let mut actions = h_flex().gap(space(2.)).child(
        Button::new(domain_id("card-link-cancel", &slot_id))
            .small()
            .design_secondary(cx)
            .design_label(tr("moment.card.link_cancel"))
            .on_click(move |_, window, cx| {
                cancel_view.emit(
                    UiEventKind::CardSlotAnswered {
                        slot_id: cancel_id.clone(),
                        outcome: CardSlotOutcome::Cancel,
                        seen: cancel_seen.clone(),
                    },
                    window,
                    cx,
                );
            }),
    );
    if host.is_some() {
        let open_slot = link.slot_id.clone();
        actions = actions.child(
            Button::new(domain_id("card-link-open", &link.slot_id))
                .small()
                .design_primary(cx)
                .design_label(tr("moment.card.link_open"))
                .on_click(move |_, window, cx| {
                    open_view.emit(
                        UiEventKind::CardSlotAnswered {
                            slot_id: open_slot.clone(),
                            outcome: CardSlotOutcome::Confirm,
                            seen: seen.clone(),
                        },
                        window,
                        cx,
                    );
                }),
        );
    }
    // A long link scrolls inside the sheet, never past the card.
    Some(
        div()
            .id(domain_id("card-link-sheet", &slot_id))
            .w_full()
            .max_h(px(420.))
            .overflow_y_scroll()
            .child(host_frame(tr("moment.card.link_title"), cx).child(sheet.child(actions)))
            .into_any_element(),
    )
}

/// Whether a card's status shows the host's verdict as pending.
pub fn pending_status(status: CardStatus) -> bool {
    matches!(status, CardStatus::AwaitingConfirmation | CardStatus::Consumed)
}
