// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! What is said next to a participant's name about who they are.
//!
//! Marks shared by the contacts and members docks, the conversation
//! header, message rows, rename lines and task cards, so the same fact
//! reads the same everywhere:
//!
//! - the **kind mark**: whether the participant is a person, an unattended
//!   AI or a bot, and how far that is proven. A kind a root-signed public
//!   profile states is "profile-signed", one only the participant claims
//!   (its card) is "claimed". The icon is the kind's own, plus a neutral
//!   signature for a signed profile, never a verified-account check: a
//!   signed profile proves what its root says, not who is behind it, and
//!   a malicious root can sign any name. A tooltip says so.
//! - the **name warning**: a name that looks like another known
//!   principal's, or mixes scripts, gets a warning with the **full**
//!   principal id beside it (never shortened: two ids may share a long
//!   prefix), with a tooltip and a copy button. Advisory only: nothing is
//!   blocked.
//! - the **stale note**: a safety number verified before this contact's
//!   identity authority changed asks to be verified again. It does not
//!   claim the keys changed.
//!
//! Nothing here is communicated by color alone: every mark has text.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::clipboard::Clipboard;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, ClickEvent, ElementId, SharedString, Window, div};
use ramflux_desktop_core::{KindClaim, Participant, ParticipantId, PeerKind, SafetyState};

use crate::i18n::{self, tr, tr_args};

/// The text of a kind mark: the kind and how far it is proven; `None`
/// when nothing was claimed.
pub fn kind_text(claim: KindClaim) -> Option<SharedString> {
    let kind = i18n::peer_kind(claim.kind()?);
    Some(if claim.is_verified() {
        tr_args("identity.kind.verified", &[("kind", &kind)])
    } else {
        tr_args("identity.kind.claimed", &[("kind", &kind)])
    })
}

/// What a kind mark's tooltip explains: what a signed profile does and
/// does not prove, or that a claim is only a claim.
pub fn kind_tooltip(claim: KindClaim) -> SharedString {
    tr(if claim.is_verified() {
        "identity.kind.verified_help"
    } else {
        "identity.kind.claimed_help"
    })
}

/// The icon of a peer kind.
fn kind_icon(kind: PeerKind) -> gpui_kit::assets::IconName {
    match kind {
        PeerKind::Human => gpui_kit::assets::IconName::User,
        PeerKind::HeadlessAi => gpui_kit::assets::IconName::Bot,
        PeerKind::Bot => gpui_kit::assets::IconName::Cpu,
    }
}

/// The kind mark of a claim: the kind's icon and text (plus a neutral
/// signature icon when a root-signed profile states it), with a tooltip on
/// what that proves. `None` when nothing was claimed.
pub fn kind_mark(id: impl Into<ElementId>, claim: KindClaim, cx: &App) -> Option<AnyElement> {
    let kind = claim.kind()?;
    let text = kind_text(claim)?;
    let help = kind_tooltip(claim);
    let color =
        if claim.is_verified() { cx.theme().foreground } else { cx.theme().muted_foreground };
    Some(
        h_flex()
            .id(id)
            .aria_label(text.clone())
            .test_support()
            .flex_none()
            .gap_1()
            .text_xs()
            .text_color(color)
            .child(Icon::new(kind_icon(kind)).xsmall())
            .child(text)
            .when(claim.is_verified(), |row| {
                row.child(Icon::new(gpui_kit::assets::IconName::Signature).xsmall())
            })
            .tooltip(move |window, cx| Tooltip::new(help.clone()).build(window, cx))
            .into_any_element(),
    )
}

/// The kind mark of a task actor. `kind` is shown only when the directory
/// itself holds a profile-signed claim of it; an actor whose kind the
/// source verified for the event without such a claim reads "Kind
/// verified"; anything else "Kind not verified".
pub fn actor_kind_mark(
    id: impl Into<ElementId>,
    kind: Option<PeerKind>,
    verified: bool,
    cx: &App,
) -> AnyElement {
    let id = id.into();
    match (kind, verified) {
        (Some(kind), true) => kind_mark(id.clone(), KindClaim::Verified(kind), cx),
        (Some(kind), false) => kind_mark(id.clone(), KindClaim::Asserted(kind), cx),
        (None, _) => None,
    }
    .unwrap_or_else(|| {
        let key =
            if verified { "identity.kind.verified_unnamed" } else { "identity.kind.unverified" };
        let text = tr(key);
        div()
            .id(id)
            .aria_label(text.clone())
            .test_support()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(text)
            .into_any_element()
    })
}

/// A full principal id: monospace, wrapping (never shortened), with a
/// tooltip and a copy button. Shown wherever the id tells two names apart.
pub fn principal_id(id: impl Into<ElementId>, principal: &ParticipantId, cx: &App) -> AnyElement {
    let id = id.into();
    let full = SharedString::from(principal.as_str().to_owned());
    let tip = tr_args("identity.id.tooltip", &[("id", principal.as_str())]);
    h_flex()
        .id(id.clone())
        .test_support()
        .min_w_0()
        .gap_1()
        .text_xs()
        .child(
            div()
                .id("principal-id-text")
                .aria_label(full.clone())
                .test_support()
                .min_w_0()
                .font_family(cx.theme().mono_font_family.clone())
                .text_color(cx.theme().muted_foreground)
                .child(full.clone())
                .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx)),
        )
        .child(
            Clipboard::new("principal-id-copy")
                .xsmall()
                .value(full)
                .accessibility_label(tr_args("identity.id.copy", &[("id", principal.as_str())])),
        )
        .into_any_element()
}

/// What is wrong with a participant's name, if anything: a short visible
/// phrase and the full sentence (with the full ids) for tooltips and
/// assistive technology.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameWarning {
    /// The short phrase ("Name resembles <id>", "Name mixes scripts").
    pub what: SharedString,
    /// The full sentence, naming the participant's own full id.
    pub text: SharedString,
}

/// The warning on `participant`'s name, if one applies.
pub fn name_warning_of(participant: &Participant) -> Option<NameWarning> {
    let warnings = &participant.name_warnings;
    let what = match (&warnings.confusable_with, warnings.mixed_script) {
        (Some(other), _) => tr_args("identity.name.confusable", &[("other", other.as_str())]),
        (None, true) => tr("identity.name.mixed_script"),
        (None, false) => return None,
    };
    let text =
        tr_args("identity.name.warning", &[("what", &what), ("id", participant.id.as_str())]);
    Some(NameWarning { what, text })
}

/// The text of a participant's name warning, if one applies.
pub fn name_warning_text(participant: &Participant) -> Option<SharedString> {
    name_warning_of(participant).map(|warning| warning.text)
}

/// The name warning of a participant: a warning icon, what is wrong with
/// the name, and the participant's full principal id (tooltip, copy).
/// `None` without a warning. Advisory: it blocks nothing and marks nothing
/// as verified.
pub fn name_warning(
    id: impl Into<ElementId>,
    participant: &Participant,
    cx: &App,
) -> Option<AnyElement> {
    let warning = name_warning_of(participant)?;
    let color = crate::tone::text(cx.theme().warning, cx);
    let text = warning.text.clone();
    Some(
        h_flex()
            .id(id)
            .aria_label(warning.text.clone())
            .test_support()
            .flex_wrap()
            .gap_1()
            .text_xs()
            .text_color(color)
            .child(Icon::new(gpui_kit::assets::IconName::TriangleAlert).xsmall())
            .child(
                div()
                    .id("name-warning-what")
                    .child(warning.what)
                    .tooltip(move |window, cx| Tooltip::new(text.clone()).build(window, cx)),
            )
            .child(div().text_color(cx.theme().muted_foreground).child("·"))
            // Two ids sit in this line: which one is theirs.
            .child(
                div().text_color(cx.theme().muted_foreground).child(tr("identity.name.their_id")),
            )
            .child(principal_id("name-warning-id", &participant.id, cx))
            .into_any_element(),
    )
}

/// The warning on a message author's name: a warning icon and a short
/// visible word ("Similar name", "Mixed scripts"), the full id, and a
/// tooltip with the whole explanation.
pub fn author_warning(
    id: impl Into<ElementId>,
    principal: &ParticipantId,
    warning: &NameWarning,
    mixed_only: bool,
    cx: &App,
) -> AnyElement {
    let text = warning.text.clone();
    let word =
        tr(if mixed_only { "identity.name.author_mixed" } else { "identity.name.author_similar" });
    h_flex()
        .id(id)
        .aria_label(warning.text.clone())
        .test_support()
        .flex_wrap()
        .gap_1()
        .text_color(crate::tone::text(cx.theme().warning, cx))
        .child(
            h_flex()
                .id("author-warning-word")
                .gap_1()
                .child(Icon::new(gpui_kit::assets::IconName::TriangleAlert).xsmall())
                .child(word)
                .tooltip(move |window, cx| Tooltip::new(text.clone()).build(window, cx)),
        )
        .child(principal_id("author-warning-id", principal, cx))
        .into_any_element()
}

/// The note of a stale verification: verify again. `None` unless stale.
pub fn stale_note(id: impl Into<ElementId>, safety: SafetyState, cx: &App) -> Option<AnyElement> {
    if !safety.is_stale() {
        return None;
    }
    let text = tr("identity.safety.stale");
    Some(
        h_flex()
            .id(id)
            .aria_label(text.clone())
            .test_support()
            .flex_none()
            .gap_1()
            .text_xs()
            .text_color(crate::tone::text(cx.theme().warning, cx))
            .child(Icon::new(gpui_kit::assets::IconName::ShieldAlert).xsmall())
            .child(tr("safety.verification_stale"))
            .into_any_element(),
    )
}

/// The Verify action's label: "Verify again" once a verification went
/// stale, else "Verify safety number".
pub fn verify_label(safety: SafetyState) -> SharedString {
    if safety.is_stale() { tr("identity.safety.verify_again") } else { tr("moment.safety.verify") }
}

/// A button handler.
pub type Click = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// A command run with the window (opens the call acknowledgement).
pub type AskCall = std::rc::Rc<dyn Fn(&mut Window, &mut App)>;

/// The one gate every call entry point goes through (the header Call
/// button and Call back on a call record): a call to a contact whose
/// verification went stale first asks for the acknowledgement (`ask`),
/// anything else places the call through `sink`. A stale contact with no
/// way to ask places nothing.
pub fn call_gate(stale: bool, ask: Option<AskCall>, sink: crate::bubble::EventSink) -> Click {
    Box::new(move |_, window, cx| match (stale, &ask) {
        (false, _) => sink.emit(ramflux_desktop_core::UiEventKind::CallStartRequested, window, cx),
        (true, Some(ask)) => ask(window, cx),
        (true, None) => {}
    })
}

/// The inline acknowledgement a stale verification asks for before a new
/// conversation, group or call: what is out of date (without claiming the
/// keys changed), Verify again and Continue. Never a refusal: Continue
/// goes ahead.
pub fn stale_ack(
    id: impl Into<ElementId>,
    prompt: SharedString,
    verify: Click,
    proceed: Click,
    cx: &App,
) -> AnyElement {
    let color = crate::tone::text(cx.theme().warning, cx);
    v_flex()
        .id(id)
        .role(gpui_kit::Role::Status)
        .aria_label(prompt.clone())
        .test_support()
        .gap_2()
        .p_2()
        .rounded(cx.theme().radius)
        .border_1()
        .border_color(cx.theme().warning)
        .child(
            h_flex()
                .gap_2()
                .text_sm()
                .child(Icon::new(gpui_kit::assets::IconName::ShieldAlert).small().text_color(color))
                .child(div().flex_1().min_w_0().child(prompt)),
        )
        .child(
            h_flex()
                .gap_2()
                .justify_end()
                .child(
                    Button::new("stale-verify")
                        .small()
                        .outline()
                        .label(tr("identity.safety.verify_again"))
                        .on_click(verify),
                )
                .child(
                    Button::new("stale-continue")
                        .small()
                        .label(tr("identity.safety.continue"))
                        .on_click(proceed),
                ),
        )
        .into_any_element()
}
