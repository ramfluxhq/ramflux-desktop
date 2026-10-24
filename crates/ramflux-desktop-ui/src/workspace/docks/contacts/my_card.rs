// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! "Your contact card": what others add to reach this account, shared out
//! of band.
//!
//! Show my card asks the daemon for the account's card (it exports a fresh
//! one each time) and shows it in place: a QR code to scan from another
//! device, the card text, and Copy buttons for the text and, when the
//! daemon offers one, the link. While the daemon answers the section says
//! so; a refusal is shown with the daemon's reason. Hide folds it away. The
//! card names this account's identity and home node only; it holds no key
//! that grants access, so showing it is not a confirmation-worthy action.
//! A card names the account's home node: without one the section says so
//! instead of offering a card.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, ClickEvent, Role, SharedString, Window, div};
use ramflux_desktop_core::{ContactCard, Store, methods};

use crate::i18n::{self, tr, tr_args};

/// Which part of the card was copied last.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Copied {
    /// The card text.
    Card,
    /// The card link.
    Link,
}

/// What the section shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CardView {
    /// The account names no home node, which a card needs: no card can be
    /// shown until it joins one.
    NoHomeNode,
    /// Folded: only Show my card.
    Hidden,
    /// Asked; the daemon has not answered.
    Preparing,
    /// The daemon refused (translated reason).
    Failed(SharedString),
    /// The card.
    Ready(ContactCard),
}

/// View-local state of the section.
#[derive(Debug, Clone, Default)]
pub struct MyCard {
    shown: bool,
    /// The store's failure count when the card was asked for.
    since: u64,
    copied: Option<Copied>,
}

impl MyCard {
    /// Unfolds the section; the caller asks the daemon for the card.
    pub fn show(&mut self, store: &Store) {
        self.shown = true;
        self.since = store.failures_total();
        self.copied = None;
    }

    /// Folds the section.
    pub fn hide(&mut self) {
        self.shown = false;
        self.copied = None;
    }

    /// Notes what was copied (the button says so until the next copy).
    pub fn copied(&mut self, part: Copied) {
        self.copied = Some(part);
    }

    /// What the section shows now. A card exported before this request is
    /// still this account's card, so it is shown while a fresh one comes.
    pub fn view(&self, store: &Store) -> CardView {
        if super::without_home_node(store) {
            return CardView::NoHomeNode;
        }
        if !self.shown {
            return CardView::Hidden;
        }
        if let Some(card) = store.own_contact_card() {
            return CardView::Ready(card.clone());
        }
        let failed = store
            .failures_since(self.since)
            .iter()
            .rev()
            .find(|f| f.method == methods::CONTACT_CARD_EXPORT);
        failed.map_or(CardView::Preparing, |failure| {
            CardView::Failed(i18n::backend_text(&failure.message))
        })
    }
}

/// A button handler.
pub type Click = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// Copies `String` (the `Copied` part) to the clipboard.
pub type CopyHandler = std::rc::Rc<dyn Fn(Copied, String, &mut Window, &mut App)>;

/// Handlers of the section's buttons.
pub struct Handlers {
    /// Show my card pressed.
    pub show: Click,
    /// Hide pressed.
    pub hide: Click,
    /// A Copy button pressed (with the text to copy and what it is).
    pub copy: CopyHandler,
}

fn copy_button(
    id: &'static str,
    part: Copied,
    text: String,
    state: &MyCard,
    copy: &CopyHandler,
) -> Button {
    let copy = copy.clone();
    let done = state.copied == Some(part);
    let label = match (part, done) {
        (_, true) => "dock.contacts.card.copied",
        (Copied::Card, false) => "dock.contacts.card.copy",
        (Copied::Link, false) => "dock.contacts.card.copy_link",
    };
    Button::new(id)
        .xsmall()
        .outline()
        .icon(if done { IconName::Check } else { IconName::Copy })
        .label(tr(label))
        .on_click(move |_, window, cx| copy(part, text.clone(), window, cx))
}

/// The section, after its title.
pub fn render(state: &MyCard, store: &Store, handlers: Handlers, cx: &App) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    let Handlers { show, hide, copy } = handlers;
    let body: AnyElement = match state.view(store) {
        CardView::Hidden => h_flex()
            .child(
                Button::new("contact-card-show")
                    .small()
                    .outline()
                    .icon(IconName::CircleUser)
                    .label(tr("dock.contacts.card.show"))
                    .on_click(show),
            )
            .into_any_element(),
        CardView::NoHomeNode => status(
            "contact-card-status",
            tr("backend.contact.no_home_node"),
            crate::tone::text(cx.theme().warning, cx),
        ),
        CardView::Preparing => {
            status("contact-card-status", tr("dock.contacts.card.preparing"), muted)
        }
        CardView::Failed(reason) => status(
            "contact-card-status",
            tr_args("dock.contacts.card.failed", &[("reason", &reason)]),
            crate::tone::text(cx.theme().danger, cx),
        ),
        CardView::Ready(card) => {
            let qr = crate::qr::render(
                "contact-card-qr",
                card.link.as_deref().unwrap_or(&card.text),
                tr("dock.contacts.card.qr"),
            );
            v_flex()
                .gap_2()
                .children(qr.map(|qr| {
                    div()
                        .p_2()
                        .rounded(cx.theme().radius)
                        .border_1()
                        .border_color(cx.theme().border)
                        .child(qr)
                }))
                .child(
                    div()
                        .id("contact-card-text")
                        .aria_label(SharedString::from(card.text.clone()))
                        .test_support()
                        .text_xs()
                        .font_family(cx.theme().mono_font_family.clone())
                        .text_color(muted)
                        .line_clamp(3)
                        .child(card.text.clone()),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .flex_wrap()
                        .child(copy_button(
                            "contact-card-copy",
                            Copied::Card,
                            card.text.clone(),
                            state,
                            &copy,
                        ))
                        .children(card.link.clone().map(|link| {
                            copy_button("contact-card-copy-link", Copied::Link, link, state, &copy)
                        }))
                        .child(div().flex_1())
                        .child(
                            Button::new("contact-card-hide")
                                .xsmall()
                                .ghost()
                                .label(tr("dock.contacts.card.hide"))
                                .on_click(hide),
                        ),
                )
                .into_any_element()
        }
    };
    v_flex()
        .id("contact-card")
        .gap_2()
        .px_3()
        .pb_2()
        .child(div().text_xs().text_color(muted).child(tr("dock.contacts.card.help")))
        .child(body)
        .into_any_element()
}

fn status(id: &'static str, text: SharedString, color: gpui_kit::Hsla) -> AnyElement {
    div()
        .id(id)
        .role(Role::Status)
        .aria_label(text.clone())
        .test_support()
        .text_xs()
        .text_color(color)
        .child(text)
        .into_any_element()
}
