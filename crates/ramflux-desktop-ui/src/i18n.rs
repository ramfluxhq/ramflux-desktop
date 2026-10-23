// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Interface strings and locale-aware labels.
//!
//! Every user-facing string comes from `locales/ui.yml` (`zh-CN` default,
//! `en` provided). Code and comments never contain translated text; keys
//! describe stable intent.

use gpui_kit::{App, SharedString};
use ramflux_desktop_core::{
    Lifecycle, MomentKind, NodeHealth, PinSource, Risk, SourceStatus, SystemLevel, Timestamp,
};

/// Locale used when nothing else is selected.
pub const DEFAULT_LOCALE: &str = "zh-CN";

/// Supported locales and the key of each locale's own name.
pub const LOCALES: [(&str, &str); 2] = [("zh-CN", "locale.chinese"), ("en", "locale.english")];

/// Looks up a translated string for the current locale.
pub fn tr(key: &str) -> SharedString {
    SharedString::from(rust_i18n::t!(key).into_owned())
}

/// Looks up a translated string and substitutes `%{name}` placeholders.
pub fn tr_args(key: &str, args: &[(&str, &str)]) -> SharedString {
    let mut text = rust_i18n::t!(key).into_owned();
    for (name, value) in args {
        text = text.replace(&format!("%{{{name}}}"), value);
    }
    SharedString::from(text)
}

/// The current locale.
pub fn locale() -> String {
    rust_i18n::locale().to_string()
}

/// Selects a locale and redraws every window.
pub fn select_locale(locale: &str, cx: &mut App) {
    rust_i18n::set_locale(locale);
    cx.refresh_windows();
}

/// Label of the source connection state.
pub fn source_status(status: SourceStatus) -> SharedString {
    tr(match status {
        SourceStatus::Connected => "status.source.connected",
        SourceStatus::Disconnected => "status.source.disconnected",
        SourceStatus::Locked => "status.source.locked",
    })
}

/// Label of a node health state.
pub fn node_health(health: NodeHealth) -> SharedString {
    tr(match health {
        NodeHealth::Unknown => "status.node.unknown",
        NodeHealth::Healthy => "status.node.healthy",
        NodeHealth::Degraded => "status.node.degraded",
        NodeHealth::Unreachable => "status.node.unreachable",
    })
}

/// Label of a node pin source.
pub fn pin_source(source: PinSource) -> SharedString {
    tr(match source {
        PinSource::Bootstrap => "node.pin.bootstrap",
        PinSource::Invite => "node.pin.invite",
        PinSource::Manual => "node.pin.manual",
    })
}

/// Label of a moment lifecycle.
pub fn lifecycle(lifecycle: Lifecycle) -> SharedString {
    tr(match lifecycle {
        Lifecycle::Pending => "lifecycle.pending",
        Lifecycle::InProgress => "lifecycle.in_progress",
        Lifecycle::NeedsMe => "lifecycle.needs_me",
        Lifecycle::Done => "lifecycle.done",
        Lifecycle::Failed => "lifecycle.failed",
        Lifecycle::Dismissed => "lifecycle.dismissed",
    })
}

/// Label of a risk level.
pub fn risk(risk: Risk) -> SharedString {
    tr(match risk {
        Risk::Low => "risk.low",
        Risk::Medium => "risk.medium",
        Risk::High => "risk.high",
        Risk::Critical => "risk.critical",
    })
}

/// Label of a system moment level.
pub fn system_level(level: SystemLevel) -> SharedString {
    tr(match level {
        SystemLevel::Info => "moment.system.info",
        SystemLevel::Warning => "moment.system.warning",
        SystemLevel::Error => "moment.system.error",
    })
}

/// Human name of a moment kind.
pub fn moment_kind(kind: &MomentKind) -> SharedString {
    tr(match kind {
        MomentKind::Message(_) => "moment.kind.message",
        MomentKind::AgentSurface(_) => "moment.kind.agent_surface",
        MomentKind::ObjectTransfer(_) => "moment.kind.object_transfer",
        MomentKind::Intervention(_) => "moment.kind.intervention",
        MomentKind::SafetyChange(_) => "moment.kind.safety_change",
        MomentKind::Governance(_) => "moment.kind.governance",
        MomentKind::ContactRequest(_) => "moment.kind.contact_request",
        MomentKind::TaskProgress(_) => "moment.kind.task_progress",
        MomentKind::Conflict(_) => "moment.kind.conflict",
        MomentKind::PrivacyReveal(_) => "moment.kind.privacy_reveal",
        MomentKind::Handoff(_) => "moment.kind.handoff",
        MomentKind::Fallback(_) => "moment.kind.fallback",
        MomentKind::System(_) => "moment.kind.system",
    })
}

/// Time of day (UTC) of a timestamp as `HH:MM`. Locale-neutral digits.
pub fn time_of_day(at: Timestamp) -> String {
    let minutes = at.millis().div_euclid(60_000);
    let hour = minutes.div_euclid(60).rem_euclid(24);
    let minute = minutes.rem_euclid(60);
    format!("{hour:02}:{minute:02}")
}

/// A scenario offset as `m:ss.t`. Locale-neutral digits.
pub fn offset(ms: u64) -> String {
    let tenths = (ms / 100) % 10;
    let seconds = (ms / 1_000) % 60;
    let minutes = ms / 60_000;
    format!("{minutes}:{seconds:02}.{tenths}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_formats_are_stable() {
        assert_eq!(time_of_day(Timestamp(1_790_812_800_000 + 3_600_000 + 5 * 60_000)), "01:05");
        assert_eq!(offset(61_250), "1:01.2");
    }
}
