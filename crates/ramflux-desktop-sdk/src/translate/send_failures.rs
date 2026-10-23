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
}
