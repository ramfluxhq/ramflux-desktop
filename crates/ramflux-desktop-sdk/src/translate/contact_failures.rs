// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Why adding, verifying or registering an identity failed, from the
//! daemon's typed `reason_code` (never from its message text).
//!
//! | `reason_code` | key |
//! |---|---|
//! | `unknown_principal`, `invalid_card`, `card_signature_invalid`, `card_expired`, `self_contact`, `already_contact`, `root_conflict`, `manifest_invalid`, `principal_id_ambiguous`, `rate_limited`, `principal_id_taken`, `device_id_taken` | `backend.contact.<reason_code>` |
//! | `root_replacement_confirmation_required` | [`key::CONTACT_ROOT_REPLACEMENT`] with `name`, `previous`, `proposed` from `details` |
//! | none, code `Unavailable` | [`key::CONTACT_UNAVAILABLE`] |
//! | `home_insecure_transport`, `home_profile_missing`, `home_profile_stale` (CONTACT-FU-02) | the home node's address cannot go on a card / no home node / refresh the home node |
//!
//! A refused `contact.card.export` is about this account's own card: its
//! typed reasons map to export wording ([`key::CONTACT_EXPORT_FAILED`] for
//! `invalid_card` and codes not known yet), never to the import text.
//!
//! Card import and export need the account's home-node profile on the
//! daemon. A card refused as `invalid_card` (or an export refused) while
//! this account names no home node is [`key::CONTACT_NO_HOME_NODE`]: the
//! account has to join its node first, and no card can fix that.

use ramflux_desktop_core::methods;
use serde_json::Value;

use super::Translator;
use super::reason::{key, keyed};
use crate::bus::BusError;

/// The confirmation the daemon asks for before a card replaces a
/// provisional (name lookup) root.
pub const ROOT_REPLACEMENT_REQUIRED: &str = "root_replacement_confirmation_required";

/// Methods whose refusals carry contact reason codes.
const CONTACT_METHODS: [&str; 6] = [
    methods::CONTACT_ADD,
    methods::CONTACT_CARD_IMPORT,
    methods::CONTACT_CARD_EXPORT,
    methods::CONTACT_ACCEPT,
    methods::SAFETY_VERIFY,
    methods::ACCOUNT_CREATE,
];

/// The key of a typed contact or registration reason.
fn reason_key(reason: &str) -> Option<&'static str> {
    Some(match reason {
        "unknown_principal" => key::CONTACT_UNKNOWN_PRINCIPAL,
        "invalid_card" => key::CONTACT_INVALID_CARD,
        "card_signature_invalid" => key::CONTACT_CARD_SIGNATURE_INVALID,
        "card_expired" => key::CONTACT_CARD_EXPIRED,
        "self_contact" => key::CONTACT_SELF,
        "already_contact" => key::CONTACT_ALREADY,
        "root_conflict" => key::CONTACT_ROOT_CONFLICT,
        "manifest_invalid" => key::CONTACT_MANIFEST_INVALID,
        "principal_id_ambiguous" => key::CONTACT_AMBIGUOUS,
        "rate_limited" => key::CONTACT_RATE_LIMITED,
        "principal_id_taken" => key::PRINCIPAL_ID_TAKEN,
        "device_id_taken" => key::DEVICE_ID_TAKEN,
        "home_insecure_transport" => key::CONTACT_HOME_INSECURE,
        "home_profile_missing" => key::CONTACT_NO_HOME_NODE,
        "home_profile_stale" => key::CONTACT_HOME_STALE,
        _ => return None,
    })
}

/// The key of a refused `contact.card.export`: the card is this account's
/// own, so a reason never says the card is someone else's or invalid. An
/// untyped refusal (`None`) is shown as the daemon wrote it.
fn export_key(reason: &str) -> &'static str {
    match reason {
        "home_insecure_transport" => key::CONTACT_HOME_INSECURE,
        "home_profile_missing" => key::CONTACT_NO_HOME_NODE,
        "home_profile_stale" => key::CONTACT_HOME_STALE,
        "rate_limited" => key::CONTACT_RATE_LIMITED,
        // `invalid_card` (today's code for a home locator a card cannot
        // carry) and any code not known yet.
        _ => key::CONTACT_EXPORT_FAILED,
    }
}

/// The root replacement to confirm, from the refusal's `details`.
fn replacement(details: Option<&Value>) -> Option<String> {
    let details = details?;
    let field = |name: &str| details[name].as_str().filter(|v| !v.is_empty());
    let (previous, proposed) =
        (field("previous_root_commitment")?, field("proposed_root_commitment")?);
    let name = field("principal_id").unwrap_or_default();
    Some(keyed(
        key::CONTACT_ROOT_REPLACEMENT,
        &[("name", name), ("previous", previous), ("proposed", proposed)],
    ))
}

impl Translator {
    /// Whether this account is known to name no home node.
    fn without_home_node(&self) -> bool {
        self.scope
            .account()
            .and_then(|id| self.accounts.get(id))
            .is_some_and(|a| a.node_id.is_none())
    }

    /// The message of a refused contact or registration effect, when the
    /// refusal is typed; `None` to show the daemon's own text.
    pub(super) fn contact_failure(&self, method: &str, error: &BusError) -> Option<String> {
        if !CONTACT_METHODS.contains(&method) {
            return None;
        }
        let BusError::Daemon { code, reason_code, details, .. } = error else { return None };
        let card = matches!(method, methods::CONTACT_CARD_IMPORT | methods::CONTACT_CARD_EXPORT);
        let home_refusal = match reason_code.as_deref() {
            Some("invalid_card") => method == methods::CONTACT_CARD_IMPORT,
            None => method == methods::CONTACT_CARD_EXPORT && code == "ValidationFailed",
            Some(_) => false,
        };
        if card && home_refusal && self.without_home_node() {
            return Some(key::CONTACT_NO_HOME_NODE.to_owned());
        }
        match reason_code.as_deref() {
            Some(reason) if method == methods::CONTACT_CARD_EXPORT => {
                Some(export_key(reason).to_owned())
            }
            Some(ROOT_REPLACEMENT_REQUIRED) => replacement(details.as_deref()),
            Some(reason) => reason_key(reason).map(str::to_owned),
            None if code == "Unavailable" => Some(key::CONTACT_UNAVAILABLE.to_owned()),
            None => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::translate::Scope;

    fn refused(reason: Option<&str>, details: Option<Value>) -> BusError {
        BusError::Daemon {
            code: "ValidationFailed".to_owned(),
            message: "refused".to_owned(),
            reason_code: reason.map(str::to_owned),
            details: details.map(Box::new),
            cause: None,
        }
    }

    #[test]
    fn typed_reasons_become_their_keys_and_untyped_ones_stay_as_written() {
        let t = Translator::new(Scope::Daemon, "rfd.sock");
        let add = methods::CONTACT_ADD;
        for (reason, expected) in [
            ("unknown_principal", key::CONTACT_UNKNOWN_PRINCIPAL),
            ("card_expired", key::CONTACT_CARD_EXPIRED),
            ("already_contact", key::CONTACT_ALREADY),
            ("rate_limited", key::CONTACT_RATE_LIMITED),
            ("principal_id_ambiguous", key::CONTACT_AMBIGUOUS),
        ] {
            assert_eq!(t.failure_message(add, &refused(Some(reason), None)), expected);
        }
        let taken = refused(Some("device_id_taken"), None);
        assert_eq!(t.failure_message(methods::ACCOUNT_CREATE, &taken), key::DEVICE_ID_TAKEN);
        let untyped = refused(None, None);
        assert_eq!(t.failure_message(add, &untyped), "refused");
        let unavailable = BusError::daemon("Unavailable", "contact lookup unavailable: dns");
        assert_eq!(t.failure_message(add, &unavailable), key::CONTACT_UNAVAILABLE);
        // Other methods keep their own explanations.
        let other = refused(Some("rate_limited"), None);
        assert_eq!(t.failure_message(methods::SEND_MESSAGE, &other), "refused");
    }

    #[test]
    fn a_card_without_a_home_node_says_so() {
        use ramflux_desktop_core::{AccountId, Timestamp};
        let mut t = Translator::new(Scope::Account(AccountId::new("acct-main")), "rfd.sock");
        let status = status_of("acct-main");
        let _ = t.fetched(&status, &json!({ "principal_id": "principal-me" }), Timestamp(0));
        let import = methods::CONTACT_CARD_IMPORT;
        let invalid = refused(Some("invalid_card"), None);
        assert_eq!(t.failure_message(import, &invalid), key::CONTACT_NO_HOME_NODE);
        let export = t.failure_message(methods::CONTACT_CARD_EXPORT, &refused(None, None));
        assert_eq!(export, key::CONTACT_NO_HOME_NODE);
        let expired = refused(Some("card_expired"), None);
        assert_eq!(t.failure_message(import, &expired), key::CONTACT_CARD_EXPIRED);
        // With a home node, an invalid card is just that.
        let _ = t.fetched(
            &status,
            &json!({ "principal_id": "principal-me", "node_id": "node-1" }),
            Timestamp(1),
        );
        assert_eq!(t.failure_message(import, &invalid), key::CONTACT_INVALID_CARD);
    }

    fn status_of(id: &str) -> crate::plan::fetch::Fetch {
        crate::plan::fetch::Fetch::AccountStatus(ramflux_desktop_core::AccountId::new(id))
    }

    #[test]
    fn an_export_refusal_is_about_this_account_own_card() {
        use ramflux_desktop_core::{AccountId, Timestamp};
        let mut t = Translator::new(Scope::Account(AccountId::new("acct-main")), "rfd.sock");
        let status = json!({ "principal_id": "principal-me", "node_id": "node-1" });
        let _ = t.fetched(&status_of("acct-main"), &status, Timestamp(0));
        let export = methods::CONTACT_CARD_EXPORT;
        for (reason, expected) in [
            ("invalid_card", key::CONTACT_EXPORT_FAILED),
            ("home_insecure_transport", key::CONTACT_HOME_INSECURE),
            ("home_profile_missing", key::CONTACT_NO_HOME_NODE),
            ("home_profile_stale", key::CONTACT_HOME_STALE),
            ("rate_limited", key::CONTACT_RATE_LIMITED),
            ("some_future_code", key::CONTACT_EXPORT_FAILED),
        ] {
            assert_eq!(t.failure_message(export, &refused(Some(reason), None)), expected);
        }
        // The same codes on an import keep the import wording.
        let import = methods::CONTACT_CARD_IMPORT;
        assert_eq!(
            t.failure_message(import, &refused(Some("invalid_card"), None)),
            key::CONTACT_INVALID_CARD
        );
        assert_eq!(
            t.failure_message(import, &refused(Some("home_insecure_transport"), None)),
            key::CONTACT_HOME_INSECURE
        );
        // Untyped: the daemon's own text.
        assert_eq!(t.failure_message(export, &refused(None, None)), "refused");
    }

    #[test]
    fn a_root_replacement_names_both_roots() {
        let t = Translator::new(Scope::Daemon, "rfd.sock");
        let details = json!({
            "principal_id": "principal-ana",
            "previous_root_commitment": "root-lookup",
            "proposed_root_commitment": "root-card",
        });
        let message = t.failure_message(
            methods::CONTACT_CARD_IMPORT,
            &refused(Some(ROOT_REPLACEMENT_REQUIRED), Some(details)),
        );
        let (name, args) = message.split_once(' ').expect("arguments");
        assert_eq!(name, key::CONTACT_ROOT_REPLACEMENT);
        let args: Value = serde_json::from_str(args).expect("json");
        assert_eq!(
            args,
            json!({ "name": "principal-ana", "previous": "root-lookup", "proposed": "root-card" })
        );
    }
}
