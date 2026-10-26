// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Public identity profiles the fake daemon holds, as the verified peer
//! kind contract has them: per principal, the name and kind a root-signed
//! profile states (`kind_verified: true`), or only what a contact card
//! asserted, and the warning-only name facts the daemon computes
//! (`mixed_script`, `display_name_confusable_with`). The fields are joined
//! into contact rows, `contact.add` answers, request rows and identity rows
//! exactly as the daemon joins them, and an authenticated rename is
//! announced as the daemon's `contact.changed` event.

use std::collections::BTreeMap;
use std::sync::Mutex;

use serde_json::{Value, json};

/// What the daemon knows of one principal's public name and kind.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Profile {
    /// The public name, when one is known.
    pub display_name: Option<String>,
    /// The kind (`human`, `headless_ai`, `bot`), when one is claimed.
    pub kind: Option<&'static str>,
    /// Whether a root-signed profile states name and kind.
    pub verified: bool,
    /// The name mixes scripts.
    pub mixed_script: bool,
    /// Another known principal whose name looks the same.
    pub confusable_with: Option<String>,
}

/// The profiles of every principal the fake daemon knows.
#[derive(Debug, Default)]
pub struct FakeProfiles {
    inner: Mutex<BTreeMap<String, Profile>>,
}

impl FakeProfiles {
    fn inner(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Profile>> {
        self.inner.lock().expect("fake profiles")
    }

    /// A root-signed profile: name and kind verified.
    pub fn verified(&self, principal: &str, name: &str, kind: &'static str) {
        let mut inner = self.inner();
        let profile = inner.entry(principal.to_owned()).or_default();
        profile.display_name = Some(name.to_owned());
        profile.kind = Some(kind);
        profile.verified = true;
    }

    /// Only what a contact card asserted (nothing verified it).
    pub fn asserted(&self, principal: &str, name: Option<&str>, kind: Option<&'static str>) {
        let mut inner = self.inner();
        let profile = inner.entry(principal.to_owned()).or_default();
        profile.display_name = name.map(str::to_owned);
        profile.kind = kind;
        profile.verified = false;
    }

    /// The daemon's warning-only name facts about `principal`.
    pub fn warn(&self, principal: &str, mixed_script: bool, confusable_with: Option<&str>) {
        let mut inner = self.inner();
        let profile = inner.entry(principal.to_owned()).or_default();
        profile.mixed_script = mixed_script;
        profile.confusable_with = confusable_with.map(str::to_owned);
    }

    /// The profile of `principal`, if any is held.
    pub fn get(&self, principal: &str) -> Option<Profile> {
        self.inner().get(principal).cloned()
    }

    /// The kind of `principal`, when one is claimed.
    pub fn kind(&self, principal: &str) -> Option<&'static str> {
        self.inner().get(principal).and_then(|p| p.kind)
    }

    /// The fields a contact row, a `contact.add` answer and a request row
    /// carry about `principal`: `display_name`, `kind_asserted`,
    /// `kind_verified`, `display_name_confusable_with`, `mixed_script`
    /// (null, null, false, null, false for an unknown principal).
    pub fn contact_fields(&self, principal: &str) -> Value {
        let profile = self.get(principal).unwrap_or_default();
        json!({
            "display_name": profile.display_name,
            "kind_asserted": profile.kind,
            "kind_verified": profile.verified,
            "display_name_confusable_with": profile.confusable_with,
            "mixed_script": profile.mixed_script,
        })
    }

    /// The fields an identity row carries about `principal`: the contact
    /// fields plus `kind` (the profile's kind, else the device set's).
    pub fn identity_fields(&self, principal: &str) -> Value {
        let mut fields = self.contact_fields(principal);
        fields["kind"] = fields["kind_asserted"].clone();
        fields
    }

    /// Joins the contact fields into `row` (an object).
    pub fn join(&self, row: &mut Value, principal: &str) {
        if let (Value::Object(row), Value::Object(fields)) = (row, self.contact_fields(principal)) {
            row.extend(fields);
        }
    }

    /// Renames `principal` through its root-signed profile; the body of the
    /// `contact.changed` event the daemon announces it with, or `None` when
    /// the principal holds no verified profile (a card label is not
    /// renamed by the daemon).
    pub fn rename(&self, principal: &str, new_name: &str) -> Option<Value> {
        let mut inner = self.inner();
        let profile = inner.get_mut(principal).filter(|p| p.verified)?;
        let old = profile.display_name.replace(new_name.to_owned());
        Some(json!({
            "principal_id": principal,
            "reason": "public_profile_renamed",
            "old_display_name": old,
            "new_display_name": new_name,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_principal_is_the_safe_default_and_a_profile_is_joined() {
        let profiles = FakeProfiles::default();
        let fields = profiles.contact_fields("p-legacy");
        assert_eq!(
            fields,
            json!({ "display_name": null, "kind_asserted": null, "kind_verified": false,
                    "display_name_confusable_with": null, "mixed_script": false })
        );
        profiles.verified("p-kai", "Kai", "human");
        profiles.warn("p-kai", true, Some("p-kay"));
        let mut row = json!({ "link_id": "l1" });
        profiles.join(&mut row, "p-kai");
        assert_eq!(row["display_name"], "Kai");
        assert_eq!(row["kind_asserted"], "human");
        assert_eq!(row["kind_verified"], true);
        assert_eq!(row["mixed_script"], true);
        assert_eq!(row["display_name_confusable_with"], "p-kay");
        assert_eq!(profiles.identity_fields("p-kai")["kind"], "human");
        profiles.asserted("p-bot", Some("Ledger"), Some("headless_ai"));
        assert_eq!(profiles.contact_fields("p-bot")["kind_verified"], false);
        assert_eq!(profiles.kind("p-bot"), Some("headless_ai"));
    }

    #[test]
    fn only_a_verified_profile_is_renamed_by_the_daemon() {
        let profiles = FakeProfiles::default();
        profiles.asserted("p-bot", Some("Ledger"), Some("bot"));
        assert_eq!(profiles.rename("p-bot", "Books"), None);
        profiles.verified("p-kai", "Kai", "human");
        let event = profiles.rename("p-kai", "Kai Ortiz").expect("announced");
        assert_eq!(event["reason"], "public_profile_renamed");
        assert_eq!(event["old_display_name"], "Kai");
        assert_eq!(event["new_display_name"], "Kai Ortiz");
        assert_eq!(profiles.contact_fields("p-kai")["display_name"], "Kai Ortiz");
    }
}
