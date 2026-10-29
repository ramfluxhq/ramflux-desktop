// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Why a send failed, in the user's terms.
//!
//! Before it encrypts or stores anything, `group.send` resolves a route for
//! every current member but the sender. When any is missing it refuses the
//! whole send with `group routes unresolved: {"group_id":…,
//! "unreachable_members":[…]}`: nothing was delivered to anyone and nothing
//! was stored. The failure names every unreachable member (by display name
//! and device when the directories know the principal, else by device id),
//! so the user sees who cannot be reached instead of a partial success.

use ramflux_desktop_core::{ParticipantId, methods};
use serde_json::Value;

use super::Translator;
use super::reason::{key, keyed};
use crate::bus::BusError;

/// The daemon's prefix of an unresolved group route set.
const UNRESOLVED: &str = "group routes unresolved: ";

/// The member devices of an unresolved group route error, if it is one.
pub fn unreachable_members(message: &str) -> Option<Vec<String>> {
    let (_, json) = message.split_once(UNRESOLVED)?;
    let value: Value = serde_json::from_str(json.trim()).ok()?;
    Some(
        value["unreachable_members"]
            .as_array()?
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
    )
}

/// The message of a typed capacity refusal: the per-call control receipt cap
/// (retryable) and the group state budget. Keyed on the typed code, never on
/// the daemon's English text.
fn typed_capacity(error: &BusError) -> Option<String> {
    let BusError::Daemon { code, reason_code, .. } = error else { return None };
    match code.as_str() {
        // The group head is held; `reason_code` says why. A known wait gets
        // its words; a stale handover says it needs a fresh one; an unknown
        // reason gets neutral words.
        crate::bus::TRANSFER_REQUIRES_SYNC => Some(
            match reason_code.as_deref() {
                Some("transfer_outcome_pending") => key::GROUP_TRANSFER_PENDING,
                Some("clock_skew_or_expired_control") => key::GROUP_CLOCK_SKEW,
                Some("head_or_removal_floor_mismatch") => key::GROUP_HANDOVER_STALE,
                Some("head_check_pending") => key::GROUP_SYNC_PENDING,
                // Any other reason (or none): neutral and final, no wait promised.
                _ => key::GROUP_NOT_READY,
            }
            .to_owned(),
        ),
        crate::bus::CALL_CONTROL_CAPACITY => Some(key::CALL_CONTROL_CAPACITY.to_owned()),
        crate::bus::GROUP_STATE_CAPACITY => Some(key::GROUP_STATE_CAPACITY.to_owned()),
        _ => None,
    }
}

impl Translator {
    /// The message of a failed effect: the daemon's, or a translated one
    /// for failures the desktop can explain.
    pub fn failure_message(&self, method: &str, error: &BusError) -> String {
        if let Some(message) = self.contact_failure(method, error) {
            return message;
        }
        if let Some(message) = typed_capacity(error) {
            return message;
        }
        if crate::capabilities::BACKUP_METHODS.contains(&method)
            && let BusError::Daemon { reason_code: Some(reason), .. } = error
            && let Some(message) = super::backup::refusal_message(reason)
        {
            return message;
        }
        if method == methods::SEND_MESSAGE
            && let BusError::Daemon { message, .. } = error
            && let Some(members) = unreachable_members(message)
        {
            let names: Vec<String> =
                members.iter().map(|device| self.member_name(device)).collect();
            return keyed(key::GROUP_ROUTES_UNRESOLVED, &[("members", &names.join(", "))]);
        }
        if method == methods::SURFACE_ACTION
            && let BusError::Daemon { code, .. } = error
            && super::surface_answers::code_of(code).is_some()
        {
            // A typed surface refusal: its text, never the English message.
            let reason = super::surface_answers::refusal_text(code);
            return keyed(key::SURFACE_ACTION_REJECTED, &[("reason", &reason)]);
        }
        if matches!(method, methods::NODE_ADD | methods::NODE_PROBE)
            && super::node_failures::local_network_permission_applies()
        {
            let message = error.message();
            if super::node_failures::is_host_unreachable(&message) {
                return key::LOCAL_NETWORK.to_owned();
            }
            if let Some(host) = super::node_failures::failed_local_request(&message) {
                return keyed(key::LOCAL_NETWORK_MAYBE, &[("host", &host)]);
            }
        }
        error.message()
    }

    fn member_name(&self, device: &str) -> String {
        match self.directory.principal_of_device(device) {
            Some(principal) => {
                let display = self
                    .reported_people
                    .get(&ParticipantId::new(principal))
                    .map_or_else(|| principal.to_owned(), |p| p.display.clone());
                format!("{display} ({device})")
            }
            None => device.to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_daemon_error_names_every_unreachable_member() {
        let message = r#"local bus error: group routes unresolved: {"group_id":"g","unreachable_members":["d-1","d-2"]}"#;
        assert_eq!(unreachable_members(message), Some(vec!["d-1".to_owned(), "d-2".to_owned()]));
        assert_eq!(unreachable_members("account not open"), None);
    }

    #[test]
    fn typed_capacity_refusals_are_explained_by_their_code() {
        use crate::translate::Scope;
        let translator = Translator::new(Scope::Daemon, "rfd.sock");
        let capped = BusError::daemon(
            "CallControlReceiptCapacity",
            "voice call control receipt capacity exhausted at 128 receipts",
        );
        assert_eq!(
            translator.failure_message(methods::CALL_ANSWER, &capped),
            key::CALL_CONTROL_CAPACITY
        );
        let held = |reason: Option<&str>| BusError::Daemon {
            code: "transfer_requires_sync".to_owned(),
            message: "Ownership transfer requires synchronization".to_owned(),
            reason_code: reason.map(str::to_owned),
            details: None,
        };
        let sync = |reason| translator.failure_message(methods::GROUP_KICK, &held(reason));
        assert_eq!(sync(Some("head_check_pending")), key::GROUP_SYNC_PENDING);
        assert_eq!(sync(Some("head_or_removal_floor_mismatch")), key::GROUP_HANDOVER_STALE);
        assert_eq!(sync(Some("directory_unavailable")), key::GROUP_NOT_READY);
        assert_eq!(sync(None), key::GROUP_NOT_READY);
        assert_eq!(sync(Some("transfer_outcome_pending")), key::GROUP_TRANSFER_PENDING);
        assert_eq!(sync(Some("clock_skew_or_expired_control")), key::GROUP_CLOCK_SKEW);
        assert!(!held(None).is_retryable() && !held(None).is_connection_lost());
        let group = BusError::daemon("GroupStateCapacity", "RemovalFences");
        assert_eq!(
            translator.failure_message(methods::GROUP_ADD_MEMBER, &group),
            key::GROUP_STATE_CAPACITY
        );
    }

    #[test]
    fn an_unreachable_node_host_on_macos_names_the_local_network_permission() {
        use crate::translate::Scope;
        let translator = Translator::new(Scope::Daemon, "rfd.sock");
        let error = BusError::daemon(
            "node",
            "node error: client record fetch failed: HostUnreachable (os error 65)",
        );
        let add = translator.failure_message(methods::NODE_ADD, &error);
        let probe = translator.failure_message(methods::NODE_PROBE, &error);
        if cfg!(target_os = "macos") {
            assert_eq!(add, key::LOCAL_NETWORK);
            assert_eq!(probe, key::LOCAL_NETWORK);
        } else {
            assert_eq!(add, error.message(), "only macOS asks for Local Network access");
        }
        assert_eq!(
            translator.failure_message(methods::SEND_MESSAGE, &error),
            error.message(),
            "only node lookups are explained this way"
        );
        let refused = BusError::daemon("node", "client record verification failed");
        assert_eq!(translator.failure_message(methods::NODE_ADD, &refused), refused.message());
        let lan = BusError::daemon(
            "node",
            "local bus error: error sending request for url \
             (http://192.168.0.10:8080/.well-known/ramflux/client)",
        );
        if cfg!(target_os = "macos") {
            assert_eq!(
                translator.failure_message(methods::NODE_ADD, &lan),
                keyed(key::LOCAL_NETWORK_MAYBE, &[("host", "192.168.0.10")])
            );
        }
    }
}
