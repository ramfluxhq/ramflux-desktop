// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The recovery screens. Each screen is one function of the recovery the
//! store reports; every message a person must not miss is a node with a
//! role (status or alert) whose name is its text, so assistive technology
//! and the tests read the same words the screen shows.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::StyledExt as _;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Context, IntoElement, Render, Role, SharedString, Window, div, rems,
};
use ramflux_desktop_core::{
    CeremonyAccount, CeremonyFailure, Networking, PHRASE_WORDS, RecoveryCeremony, RecoveryPhase,
    check_phrase,
};

use super::{RecoveryPanel, RecoveryPending, Screen};
use crate::actions::RECOVERY_CONTEXT;
use crate::entry::parts::{announced, clock, muted, ringed, warning};
use crate::i18n::{tr, tr_args};

/// The fingerprint in groups of four, so it can be compared by eye.
fn grouped(anchor: &str) -> String {
    let chars: Vec<char> = anchor.chars().collect();
    chars.chunks(4).map(|group| group.iter().collect::<String>()).collect::<Vec<_>>().join(" ")
}

/// The message of a recovery that ended without a change.
fn failure_key(failure: CeremonyFailure) -> &'static str {
    match failure {
        CeremonyFailure::Disabled => "recovery.ended.disabled",
        CeremonyFailure::ConfirmationFailed => "recovery.ended.mismatch",
        CeremonyFailure::VaultUnavailable => "recovery.ended.vault",
        CeremonyFailure::Expired => "recovery.ended.expired",
        // A recovery never has a custodian: a ceremony of another kind.
        CeremonyFailure::Invalid | CeremonyFailure::CustodianRefused => "recovery.ended.invalid",
    }
}

impl RecoveryPanel {
    /// The account's name as the page lists it (its id when unknown).
    fn name(&self, cx: &App) -> String {
        let Some(id) = &self.account else { return String::new() };
        self.store
            .read(cx)
            .entry_store()
            .account(id)
            .map_or_else(|| id.to_string(), |account| account.label.clone())
    }

    fn heading(&self, text: SharedString, window: &Window, cx: &App) -> AnyElement {
        ringed(
            div()
                .id("recovery-heading")
                .role(Role::Heading)
                .aria_label(text.clone())
                .track_focus(&self.heading_focus)
                .test_support()
                .px_1()
                .rounded(cx.theme().radius)
                .text_lg()
                .font_semibold()
                .child(text),
            self.heading_focus.is_focused(window),
            window,
            cx,
        )
        .into_any_element()
    }

    fn cancel_button(&self, cx: &mut Context<Self>) -> Button {
        let waiting = self.pending == Some(RecoveryPending::Cancel);
        Button::new("recovery-cancel")
            .outline()
            .label(tr("recovery.cancel"))
            .loading(waiting)
            .disabled(waiting || self.pending == Some(RecoveryPending::Confirm))
            .on_click(cx.listener(|this, _, window, cx| this.cancel(window, cx)))
    }

    fn countdown(&self, record: &RecoveryCeremony, cx: &App) -> AnyElement {
        let left = clock(record.remaining_ms(self.now(cx)));
        let text = tr_args("recovery.expires_in", &[("time", &left)]);
        div()
            .id("recovery-countdown")
            .role(Role::Timer)
            .aria_label(text.clone())
            .test_support()
            .child(muted(text, cx))
            .into_any_element()
    }

    fn intro_screen(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let name = self.name(cx);
        let waiting = self.pending == Some(RecoveryPending::Begin) || self.abandon.is_some();
        v_flex()
            .id("recovery-intro")
            .test_support()
            .gap_3()
            .child(self.heading(tr_args("recovery.intro.title", &[("name", &name)]), window, cx))
            .child(warning("recovery-warning", "recovery.intro.warning"))
            .child(announced(
                "recovery-help",
                Role::Status,
                tr("recovery.intro.help"),
                muted(tr("recovery.intro.help"), cx),
            ))
            .children(waiting.then(|| {
                announced(
                    "recovery-starting",
                    Role::Status,
                    tr("recovery.intro.pending"),
                    h_flex()
                        .gap_2()
                        .items_center()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(Spinner::new().small())
                        .child(tr("recovery.intro.pending")),
                )
            }))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("recovery-continue")
                            .primary()
                            .label(tr("recovery.continue"))
                            .loading(waiting)
                            .disabled(self.pending.is_some() || self.abandon.is_some())
                            .on_click(cx.listener(|this, _, window, cx| this.start(window, cx))),
                    )
                    .child(self.cancel_button(cx)),
            )
            .into_any_element()
    }

    fn check_screen(
        &self,
        record: &RecoveryCeremony,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let shown = grouped(&record.anchor);
        let shown_label = tr_args("recovery.fingerprint.shown", &[("value", &shown)]);
        let typed_fingerprint = !self.fingerprint.read(cx).value().trim().is_empty();
        let matches = self.fingerprint_matches(cx);
        let fingerprint_line = if matches {
            Some(announced(
                "recovery-fingerprint-state",
                Role::Status,
                tr("recovery.fingerprint.match"),
                div()
                    .text_sm()
                    .text_color(cx.theme().success)
                    .child(tr("recovery.fingerprint.match")),
            ))
        } else if typed_fingerprint {
            Some(announced(
                "recovery-fingerprint-state",
                Role::Alert,
                tr("recovery.fingerprint.mismatch"),
                div()
                    .text_sm()
                    .text_color(cx.theme().danger)
                    .child(tr("recovery.fingerprint.mismatch")),
            ))
        } else {
            None
        };
        // The phrase is read while it is typed, to count and to name the
        // first word that cannot be one; nothing of it is kept.
        let check = check_phrase(self.phrase.read(cx).value().as_ref());
        let phrase_line = if check.tokens == 0 {
            None
        } else if let Some(number) = check.first_unknown {
            let text = tr_args("recovery.phrase.unknown", &[("number", &number.to_string())]);
            Some(announced(
                "recovery-phrase-state",
                Role::Alert,
                text.clone(),
                div().text_sm().text_color(cx.theme().danger).child(text),
            ))
        } else if check.tokens > PHRASE_WORDS {
            Some(announced(
                "recovery-phrase-state",
                Role::Alert,
                tr("recovery.phrase.too_many"),
                div().text_sm().text_color(cx.theme().danger).child(tr("recovery.phrase.too_many")),
            ))
        } else {
            let text = if check.complete() {
                tr("recovery.phrase.ready")
            } else {
                tr_args("recovery.phrase.count", &[("count", &check.resolved.to_string())])
            };
            Some(announced("recovery-phrase-state", Role::Status, text.clone(), muted(text, cx)))
        };
        let field = |label: &'static str, input: &gpui_kit::Entity<_>, id: &'static str, masked| {
            let input = Input::new(input).id(id).aria_label(tr(label));
            v_flex().gap_1().child(muted(tr(label), cx)).child(if masked {
                input.mask_toggle()
            } else {
                input
            })
        };
        v_flex()
            .id("recovery-check")
            .test_support()
            .gap_3()
            .child(self.heading(tr("recovery.check.title"), window, cx))
            .child(announced(
                "recovery-fingerprint-help",
                Role::Status,
                tr("recovery.fingerprint.help"),
                muted(tr("recovery.fingerprint.help"), cx),
            ))
            .child(announced(
                "recovery-anchor",
                Role::Status,
                shown_label,
                div()
                    .px_3()
                    .py_2()
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded(cx.theme().radius)
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_base()
                    .child(shown),
            ))
            .child(field(
                "recovery.fingerprint.label",
                &self.fingerprint,
                "recovery-fingerprint-input",
                false,
            ))
            .children(fingerprint_line)
            .child(field("recovery.phrase.label", &self.phrase, "recovery-phrase-input", true))
            .children(phrase_line)
            .child(field(
                "recovery.passphrase.label",
                &self.passphrase,
                "recovery-passphrase-input",
                true,
            ))
            .child(muted(tr("recovery.passphrase.help"), cx))
            .child(self.countdown(record, cx))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("recovery-submit")
                            .primary()
                            .label(tr("recovery.submit"))
                            .disabled(!self.can_recover(cx))
                            .on_click(cx.listener(|this, _, window, cx| this.recover(window, cx))),
                    )
                    .child(self.cancel_button(cx)),
            )
            .into_any_element()
    }

    /// The one attempt was sent: nothing is typed any more, and the answer
    /// is awaited.
    fn checking_screen(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .id("recovery-checking")
            .test_support()
            .gap_3()
            .child(self.heading(tr("recovery.check.title"), window, cx))
            .child(ringed(
                div()
                    .id("recovery-checking-text")
                    .role(Role::Status)
                    .aria_label(tr("recovery.confirming"))
                    .track_focus(&self.status_focus)
                    .test_support()
                    .p_1()
                    .rounded(cx.theme().radius)
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(Spinner::new().small())
                            .child(tr("recovery.confirming")),
                    ),
                self.status_focus.is_focused(window),
                window,
                cx,
            ))
            .into_any_element()
    }

    fn done_screen(
        &self,
        account: &CeremonyAccount,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let live = account.networking == Networking::Live;
        let (state, state_text) = if live {
            ("ceremony.done.live_state", "ceremony.done.live_state_text")
        } else {
            ("ceremony.done.state", "ceremony.done.state_text")
        };
        let text = tr_args("recovery.done.text", &[("name", &account.display_name)]);
        v_flex()
            .id("recovery-done")
            .test_support()
            .gap_3()
            .child(self.heading(tr("recovery.done.title"), window, cx))
            .child(announced(
                "recovery-state",
                Role::Status,
                tr(state_text),
                h_flex().child(if live {
                    Tag::success().child(tr(state))
                } else {
                    Tag::warning().child(tr(state))
                }),
            ))
            .child(announced(
                "recovery-done-text",
                Role::Status,
                text.clone(),
                Alert::success("recovery-done-alert", text).small(),
            ))
            .child(
                h_flex().child(
                    Button::new("recovery-done-close")
                        .primary()
                        .label(tr("recovery.done.close"))
                        .on_click(cx.listener(|this, _, window, cx| this.close(window, cx))),
                ),
            )
            .into_any_element()
    }

    /// The recovery ended without a change.
    fn ended_screen(
        &self,
        key: &'static str,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let text = tr(key);
        v_flex()
            .id("recovery-ended")
            .test_support()
            .gap_3()
            .child(ringed(
                div()
                    .id("recovery-ended-text")
                    .role(Role::Alert)
                    .aria_label(text.clone())
                    .track_focus(&self.alert_focus)
                    .test_support()
                    .p_1()
                    .rounded(cx.theme().radius)
                    .child(Alert::error("recovery-ended-alert", text).small()),
                self.alert_focus.is_focused(window),
                window,
                cx,
            ))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("recovery-restart")
                            .primary()
                            .label(tr("recovery.restart"))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.start_over(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("recovery-close")
                            .outline()
                            .label(tr("recovery.close"))
                            .on_click(cx.listener(|this, _, window, cx| this.close(window, cx))),
                    ),
            )
            .into_any_element()
    }

    /// Moves keyboard focus when a screen is first drawn: to the first
    /// input to type in; to the heading, the pending line or the error of a
    /// screen with nothing to type. Never to a button: the first press after
    /// a screen appears must not be able to send or cancel anything.
    fn focus_screen(&mut self, screen: Screen, window: &mut Window, cx: &mut Context<Self>) {
        if self.focused_screen == Some(screen) {
            return;
        }
        self.focused_screen = Some(screen);
        let panel = cx.weak_entity();
        window.defer(cx, move |window, cx| {
            panel
                .update(cx, |panel, cx| match screen {
                    Screen::Check => {
                        let input = panel.fingerprint.clone();
                        input.update(cx, |input, cx| input.focus(window, cx));
                    }
                    Screen::Checking => window.focus(&panel.status_focus, cx),
                    Screen::Ended => window.focus(&panel.alert_focus, cx),
                    Screen::Intro | Screen::Done => window.focus(&panel.heading_focus, cx),
                })
                .ok();
        });
    }
}

impl Render for RecoveryPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let record = self.record(cx);
        let (screen, body) = match record {
            // The five minutes passed: the expiry stays, whatever the
            // recovery became since it was let go.
            Some(_) if self.expired => {
                (Screen::Ended, self.ended_screen("recovery.ended.expired", window, cx))
            }
            None => (Screen::Intro, self.intro_screen(window, cx)),
            Some(record) => match record.phase.clone() {
                RecoveryPhase::Prepared if self.pending == Some(RecoveryPending::Confirm) => {
                    (Screen::Checking, self.checking_screen(window, cx))
                }
                RecoveryPhase::Prepared => (Screen::Check, self.check_screen(&record, window, cx)),
                RecoveryPhase::Recovered(account) => {
                    (Screen::Done, self.done_screen(&account, window, cx))
                }
                RecoveryPhase::Failed(failure) => {
                    (Screen::Ended, self.ended_screen(failure_key(failure), window, cx))
                }
                RecoveryPhase::Cancelled => {
                    (Screen::Ended, self.ended_screen("recovery.ended.cancelled", window, cx))
                }
            },
        };
        self.focus_screen(screen, window, cx);
        v_flex()
            .id("recovery")
            .key_context(RECOVERY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_escape))
            .test_support()
            .w_full()
            .max_w(rems(40.))
            .gap_3()
            .child(body)
    }
}
