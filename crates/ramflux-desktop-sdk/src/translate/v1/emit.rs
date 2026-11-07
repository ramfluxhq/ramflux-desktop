// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The one place a v1 projection reports a moment.
//!
//! A message whose revision is below the highest edit applied to it (the
//! base the content replay rebuilt after a lock or an eviction, before the
//! edit is replayed) is a *pending base*: it is retained, so the edit can
//! build on it, but it is never reported on any path until the edit has been
//! applied again. Reporting it would show the text the edit removed.
//!
//! Every path that reports a moment goes through [`emit`]; the guard test
//! below fails if any other v1 file calls `report_moment` directly.

use ramflux_desktop_core::{DomainEvent, Moment, MomentKind};

use crate::translate::Translator;

/// Whether `moment` is a message older than an edit already applied to it.
fn is_pending_base(translator: &Translator, moment: &Moment) -> bool {
    let MomentKind::Message(payload) = &moment.kind else { return false };
    !payload.tombstone
        && translator
            .v1
            .fences
            .get(&moment.id)
            .is_some_and(|fence| payload.revision < fence.revision)
}

/// Reports `moment`, unless it is a pending base. A skipped moment forgets
/// what was last reported for it, so the edit that replaces it is published
/// (and a requested body reload is still honoured then).
pub(super) fn emit(translator: &mut Translator, moment: Moment) -> Option<DomainEvent> {
    if is_pending_base(translator, &moment) {
        translator.moments.remove(&moment.id);
        return None;
    }
    translator.report_moment(moment)
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_emit_reports_moments() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src/translate/v1");
        let mut offenders = Vec::new();
        for entry in std::fs::read_dir(dir).expect("v1 directory") {
            let path = entry.expect("entry").path();
            if path.extension().is_none_or(|e| e != "rs") || path.ends_with("emit.rs") {
                continue;
            }
            let source = std::fs::read_to_string(&path).expect("source");
            if source.contains(".report_moment(") {
                offenders.push(path.display().to_string());
            }
        }
        assert!(offenders.is_empty(), "report a moment through emit(): {offenders:?}");
    }
}
