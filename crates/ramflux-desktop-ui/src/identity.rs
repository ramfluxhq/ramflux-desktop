// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! What is said next to a participant's name about who they are.
//!
//! Three marks, shared by the contacts and members docks, the conversation
//! header, message rows and task cards, so the same fact reads the same
//! everywhere:
//!
//! - the **kind mark**: whether the participant is a person, a headless AI
//!   or a bot, and how far that is proven. A kind a root-signed public
//!   profile states is "verified" and carries a check; a kind only the
//!   participant claims (its card) is "claimed", quiet text. The mark
//!   belongs to the kind claim, never to the name: a malicious root can
//!   sign any name, so it is never drawn on the name itself.
//! - the **name warning**: a name that looks like another known
//!   principal's, or mixes scripts, gets a warning badge with the
//!   principal id beside it. Advisory only: nothing is blocked.
//! - the **stale note**: a safety number that was verified against an
//!   authority that changed since asks to be verified again.
//!
//! Nothing here is communicated by color alone: every mark has text.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, ElementId, SharedString, div};
use ramflux_desktop_core::{KindClaim, Participant, PeerKind, SafetyState};

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

/// The icon of a peer kind.
fn kind_icon(kind: PeerKind) -> gpui_kit::assets::IconName {
    match kind {
        PeerKind::Human => gpui_kit::assets::IconName::User,
        PeerKind::HeadlessAi => gpui_kit::assets::IconName::Bot,
        PeerKind::Bot => gpui_kit::assets::IconName::Cpu,
    }
}

/// The kind mark of a claim: the kind's icon and text, with a check and
/// plain foreground when a root-signed profile proves it, muted text when
/// it is only claimed. `None` when nothing was claimed.
pub fn kind_mark(id: impl Into<ElementId>, claim: KindClaim, cx: &App) -> Option<AnyElement> {
    let kind = claim.kind()?;
    let text = kind_text(claim)?;
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
                row.child(Icon::new(gpui_kit::assets::IconName::BadgeCheck).xsmall())
            })
            .into_any_element(),
    )
}

/// The kind mark of a task actor: its profile kind, verified for its role
/// or not; without a kind, "kind not verified".
pub fn actor_kind_mark(
    id: impl Into<ElementId>,
    kind: Option<PeerKind>,
    verified: bool,
    cx: &App,
) -> AnyElement {
    let claim = match (kind, verified) {
        (Some(kind), true) => KindClaim::Verified(kind),
        (Some(kind), false) => KindClaim::Asserted(kind),
        (None, _) => KindClaim::None,
    };
    kind_mark(id, claim, cx).unwrap_or_else(|| unverified_mark(cx))
}

/// "Kind not verified", for an actor whose kind nothing states.
fn unverified_mark(cx: &App) -> AnyElement {
    div()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(tr("identity.kind.unverified"))
        .into_any_element()
}

/// The text of a participant's name warning, if one applies: what is
/// wrong with the name, and the principal id to tell them apart by.
pub fn name_warning_text(participant: &Participant) -> Option<SharedString> {
    let warnings = &participant.name_warnings;
    let what = match (&warnings.confusable_with, warnings.mixed_script) {
        (Some(other), _) => {
            tr_args("identity.name.confusable", &[("other", other.short().as_str())])
        }
        (None, true) => tr("identity.name.mixed_script"),
        (None, false) => return None,
    };
    Some(tr_args("identity.name.warning", &[("what", &what), ("id", participant.id.as_str())]))
}

/// The name warning badge of a participant: a warning icon, what is wrong
/// with the name, and the principal id in monospace. `None` without a
/// warning. Advisory: it blocks nothing and marks nothing as verified.
pub fn name_warning(
    id: impl Into<ElementId>,
    participant: &Participant,
    cx: &App,
) -> Option<AnyElement> {
    let text = name_warning_text(participant)?;
    let warnings = &participant.name_warnings;
    let what = match (&warnings.confusable_with, warnings.mixed_script) {
        (Some(other), _) => {
            tr_args("identity.name.confusable", &[("other", other.short().as_str())])
        }
        _ => tr("identity.name.mixed_script"),
    };
    let color = crate::tone::text(cx.theme().warning, cx);
    Some(
        h_flex()
            .id(id)
            .aria_label(text)
            .test_support()
            .flex_wrap()
            .gap_1()
            .text_xs()
            .text_color(color)
            .child(Icon::new(gpui_kit::assets::IconName::TriangleAlert).xsmall())
            .child(what)
            .child(
                div()
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_color(cx.theme().muted_foreground)
                    .child(participant.id.short()),
            )
            .into_any_element(),
    )
}

/// The note of a stale verification: the safety number was verified, the
/// authority changed since; verify again. `None` unless stale.
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
            .child(text)
            .into_any_element(),
    )
}

/// The Verify action's label: "Verify again" once a verification went
/// stale, else "Verify safety number".
pub fn verify_label(safety: SafetyState) -> SharedString {
    if safety.is_stale() { tr("identity.safety.verify_again") } else { tr("moment.safety.verify") }
}
