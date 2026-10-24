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

impl Translator {
    /// The message of a failed effect: the daemon's, or a translated one
    /// for failures the desktop can explain.
    pub fn failure_message(&self, method: &str, error: &BusError) -> String {
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
    fn an_unreachable_node_host_on_macos_names_the_local_network_permission() {
        use crate::translate::Scope;
        let translator = Translator::new(Scope::Daemon, "rfd.sock");
        let error = BusError::Daemon {
            code: "node".to_owned(),
            message: "node error: client record fetch failed: HostUnreachable (os error 65)"
                .to_owned(),
        };
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
        let refused = BusError::Daemon {
            code: "node".to_owned(),
            message: "client record verification failed".to_owned(),
        };
        assert_eq!(translator.failure_message(methods::NODE_ADD, &refused), refused.message());
        let lan = BusError::Daemon {
            code: "node".to_owned(),
            message: "local bus error: error sending request for url \
                      (http://192.168.0.10:8080/.well-known/ramflux/client)"
                .to_owned(),
        };
        if cfg!(target_os = "macos") {
            assert_eq!(
                translator.failure_message(methods::NODE_ADD, &lan),
                keyed(key::LOCAL_NETWORK_MAYBE, &[("host", "192.168.0.10")])
            );
        }
    }
}
