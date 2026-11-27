// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Host-owned presentation: the approval card, the modal sheet and the link
//! confirmation. The verified facts come only from the host slot the runtime
//! stamped (its task binding), never from tree text. The card's explanation
//! is shown as the author's words, labelled as such.
//!
//! A link sheet shows the host (on its own line), the exact URL in a
//! scrollable block, the author's name and a privacy note, with Cancel and a
//! single tap Open. Only an https link can be opened; any other link shows a
//! refusal and Cancel. The browser opens only after the runtime answers the
//! slot `opened` for the URL the sheet showed (the app does that, not this
//! module).

use gpui_kit::component::StyledExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, IntoElement, Window, div, px};
use ramflux_desktop_core::{
    CardInput, CardNode, CardRecord, CardSlotOutcome, CardStatus, UiEventKind, https_host,
};
use ramflux_desktop_navigation::direction::h_flex;
use ramflux_desktop_navigation::scale::{space, text_caption, text_secondary, text_subheading};

use super::{CardSnapshot, View, attr_text, shown};
use crate::i18n::{tr, tr_args};
use crate::ids::domain_id;
use crate::style::DesignButton as _;

/// An approval card: the author's explanation, the verified task binding and
/// Deny / Confirm while the runtime waits for the host (then "Waiting for
/// confirmation" until the signed update).
pub fn approval(view: &View<'_>, node: &CardNode, window: &mut Window, cx: &mut App) -> AnyElement {
    let _ = window;
    let slot = view.tree.host_slots().iter().find(|s| {
        s.key == node.key && s.request.get("kind").and_then(|k| k.as_str()) == Some("approval")
    });
    let explanation = attr_text(node, "explanation");
    let binding = slot.and_then(|s| s.request.get("task_binding")).and_then(|b| {
        let task = shown(b.get("task_id")?.as_str()?);
        let step = shown(b.get("step_id")?.as_str()?);
        Some(tr_args("moment.card.approval_task", &[("task", &task), ("step", &step)]))
    });
    let mut col = v_flex()
        .w_full()
        .gap(space(2.))
        .p(space(3.))
        .rounded(cx.theme().radius)
        .border_1()
        .border_color(cx.theme().border)
        .child(
            h_flex()
                .gap(space(2.))
                .items_center()
                .child(Icon::new(IconName::TriangleAlert).small().text_color(cx.theme().warning))
                .child(
                    div()
                        .text_size(text_secondary())
                        .font_semibold()
                        .child(tr("moment.card.approval_title")),
                ),
        );
    if let Some(binding) = binding {
        col = col.child(div().text_size(text_caption()).child(binding));
    }
    if !explanation.is_empty() {
        col = col.child(
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
    let decision: AnyElement = match slot {
        _ if pending_status(view.status) => div()
            .text_size(text_caption())
            .text_color(cx.theme().muted_foreground)
            .child(tr("moment.card.waiting_confirmation"))
            .into_any_element(),
        Some(slot) if view.enabled(node) => {
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
        _ => div().into_any_element(),
    };
    col.child(decision).into_any_element()
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
/// the privacy note, with Cancel and Open. A link that is not https is
/// refused: only Cancel is offered.
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
    let url = shown(&link.url);
    let host = https_host(&link.url).map(shown);
    let mut sheet = v_flex()
        .w_full()
        .gap(space(2.))
        .p(space(3.))
        .rounded(cx.theme().radius)
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().background)
        .child(
            div().text_size(text_subheading()).font_semibold().child(tr("moment.card.link_title")),
        );
    match &host {
        Some(host) => {
            sheet =
                sheet.child(div().text_size(text_secondary()).font_semibold().child(host.clone()));
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
                .id(domain_id("card-link-url", &slot_id))
                .max_h(px(120.))
                .overflow_y_scroll()
                .text_size(text_secondary())
                .font_family(cx.theme().mono_font_family.clone())
                .child(url),
        )
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
            .child(sheet.child(actions))
            .into_any_element(),
    )
}

/// Whether a card's status shows the host's verdict as pending.
pub fn pending_status(status: CardStatus) -> bool {
    matches!(status, CardStatus::AwaitingConfirmation | CardStatus::Consumed)
}
