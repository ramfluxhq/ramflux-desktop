// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Reasons and failure messages as locale keys, with arguments.
//!
//! A message the desktop writes itself is a locale key (`backend.*`) the UI
//! translates. When it needs values (the members a group send could not
//! reach), the key is followed by one space and a JSON object of string
//! arguments: `backend.failure.group_routes_unresolved {"members":"…"}`. The
//! UI substitutes them into the translated text; anything else is shown as
//! the daemon wrote it.

use serde_json::{Map, Value};

/// Refusal and failure keys the planner and translator use.
pub mod key {
    /// The conversation has no active route (none captured, or its device
    /// is no longer verified).
    pub const NO_ROUTE: &str = "backend.gap.no_route";
    /// The group conversation is not linked to a group.
    pub const NO_GROUP_LINK: &str = "backend.gap.no_group_link";
    /// No verified device (signing key and target) is listed for the person.
    pub const NO_MEMBER_ROUTE: &str = "backend.gap.no_member_route";
    /// The participant has no member device in the group.
    pub const NOT_A_MEMBER_DEVICE: &str = "backend.gap.no_member_device";
    /// The identity commitment of the contact is not listed.
    pub const NO_COMMITMENT: &str = "backend.gap.no_commitment";
    /// The account's node relay is not known.
    pub const NO_RELAY: &str = "backend.gap.no_relay";
    /// This account's own ids are not known yet.
    pub const NO_SELF: &str = "backend.gap.no_self";
    /// The object's direction is not known.
    pub const NO_DIRECTION: &str = "backend.gap.no_direction";
    /// An attachment over the one-frame limit (`limit`).
    pub const ATTACHMENT_TOO_LARGE: &str = "backend.failure.attachment_too_large";
    /// The file could not be read or written (`reason`).
    pub const FILE: &str = "backend.failure.file";
    /// A group send reached no one: `members` could not be resolved.
    pub const GROUP_ROUTES_UNRESOLVED: &str = "backend.failure.group_routes_unresolved";
    /// The node's host is unreachable because Local Network access was not
    /// allowed (macOS privacy).
    pub const LOCAL_NETWORK: &str = "backend.failure.local_network";
    /// A request to `host` on the local network failed without a reason
    /// the daemon passed on; Local Network access may be missing.
    pub const LOCAL_NETWORK_MAYBE: &str = "backend.failure.local_network_maybe";
    /// A multi-step effect stopped (`done` of `total` applied, `reason`).
    pub const PARTIAL: &str = "backend.failure.partial";
    /// A new conversation: the identity directory does not list the
    /// contact (`name`) yet.
    pub const START_NO_IDENTITY: &str = "backend.failure.start_no_identity";
    /// A new conversation: no device of the contact (`name`) has a
    /// delivery target.
    pub const START_NO_TARGET: &str = "backend.failure.start_no_target";
    /// A new conversation: the contact's (`name`) devices have targets but
    /// no verified signing key.
    pub const START_UNVERIFIED_KEY: &str = "backend.failure.start_unverified_key";

    /// The daemon did not send a surface action (`reason`).
    pub const SURFACE_ACTION_REJECTED: &str = "backend.failure.surface_action_rejected";
    /// The daemon refused a surface request with a code the desktop does
    /// not know (`code`).
    pub const SURFACE_REFUSED: &str = "backend.surface.refused";
    /// Typed surface refusals (the daemon's `reason_code`), one key each.
    /// The surface had a newer version than the one shown.
    pub const SURFACE_STALE_VERSION: &str = "backend.surface.stale_version";
    /// The surface's content changed after it was shown.
    pub const SURFACE_STALE_HASH: &str = "backend.surface.stale_hash";
    /// Only the surface's author may do it.
    pub const SURFACE_NOT_AUTHOR: &str = "backend.surface.not_author";
    /// The surface is up to date (nothing to request).
    pub const SURFACE_NOT_OUT_OF_SYNC: &str = "backend.surface.not_out_of_sync";
    /// Asked too recently.
    pub const SURFACE_RATE_LIMITED: &str = "backend.surface.rate_limited";
    /// Too large to send.
    pub const SURFACE_TOO_LARGE: &str = "backend.surface.too_large";
    /// The daemon no longer holds the surface.
    pub const SURFACE_NOT_FOUND: &str = "backend.surface.not_found";
    /// The surface is out of sync.
    pub const SURFACE_OUT_OF_SYNC: &str = "backend.surface.out_of_sync";
    /// Another device of the account takes the surface's actions.
    pub const SURFACE_NOT_ACTION_EXECUTOR: &str = "backend.surface.not_action_executor";
    /// The action was used already.
    pub const SURFACE_ALREADY_USED: &str = "backend.surface.already_used";
    /// The surface is closed.
    pub const SURFACE_CLOSED: &str = "backend.surface.closed";
    /// Not permitted on this surface.
    pub const SURFACE_NOT_PERMITTED: &str = "backend.surface.not_permitted";
    /// The daemon could not accept the request.
    pub const SURFACE_INVALID_REQUEST: &str = "backend.surface.invalid_request";

    /// A new group: no verified device (signing key and delivery target)
    /// is listed for the member (`name`), so the group is not created.
    pub const GROUP_MEMBER_ROUTE: &str = "backend.failure.group_member_route";

    /// Contact refusal `unknown_principal`: No one is registered under this id on the home node.
    pub const CONTACT_UNKNOWN_PRINCIPAL: &str = "backend.contact.unknown_principal";
    /// Contact refusal `invalid_card`: This is not a contact card for your home node.
    pub const CONTACT_INVALID_CARD: &str = "backend.contact.invalid_card";
    /// Contact refusal `card_signature_invalid`: The card's signature does not match its identity.
    pub const CONTACT_CARD_SIGNATURE_INVALID: &str = "backend.contact.card_signature_invalid";
    /// Contact refusal `card_expired`: The card has expired.
    pub const CONTACT_CARD_EXPIRED: &str = "backend.contact.card_expired";
    /// Contact refusal `self_contact`: That is this account itself.
    pub const CONTACT_SELF: &str = "backend.contact.self_contact";
    /// Contact refusal `already_contact`: Already a contact.
    pub const CONTACT_ALREADY: &str = "backend.contact.already_contact";
    /// Contact refusal `root_conflict`: The contact's identity changed while it was being confirmed.
    pub const CONTACT_ROOT_CONFLICT: &str = "backend.contact.root_conflict";
    /// Contact refusal `manifest_invalid`: The contact's published identity could not be verified.
    pub const CONTACT_MANIFEST_INVALID: &str = "backend.contact.manifest_invalid";
    /// Contact refusal `principal_id_ambiguous`: Several identities hold this id on the node.
    pub const CONTACT_AMBIGUOUS: &str = "backend.contact.principal_id_ambiguous";
    /// Re-asking a contact refused while their decline cools down (24 h;
    /// CONTACT-02 r2): `name`.
    pub const CONTACT_REQUEST_COOLDOWN: &str = "backend.contact.request_cooldown";
    /// Contact request refusal `request_closed`: answered or expired already.
    pub const CONTACT_REQUEST_CLOSED: &str = "backend.contact.request_closed";
    /// Contact request refusal `request_not_found`: no such request.
    pub const CONTACT_REQUEST_NOT_FOUND: &str = "backend.contact.request_not_found";
    /// Contact request refusal `invalid_request_note`: the note is not usable.
    pub const CONTACT_REQUEST_NOTE: &str = "backend.contact.invalid_request_note";
    /// Contact refusal `rate_limited`: Too many lookups.
    pub const CONTACT_RATE_LIMITED: &str = "backend.contact.rate_limited";
    /// Contact refusal `principal_id_taken`: The id is registered to another identity.
    pub const PRINCIPAL_ID_TAKEN: &str = "backend.contact.principal_id_taken";
    /// Contact refusal `device_id_taken`: The device id is registered to another identity.
    pub const DEVICE_ID_TAKEN: &str = "backend.contact.device_id_taken";
    /// The daemon's media session of a call could not be read.
    pub const CALL_MEDIA: &str = "backend.call.media";
    /// Contact refusal `unavailable`: The home node could not be reached.
    pub const CONTACT_UNAVAILABLE: &str = "backend.contact.unavailable";
    /// Contact refusal `root_replacement`: The card names another identity than the lookup (`name`, `previous`, `proposed`).
    pub const CONTACT_ROOT_REPLACEMENT: &str = "backend.contact.root_replacement";
    /// Contact refusal `no_home_node`: The account has no home node profile.
    pub const CONTACT_NO_HOME_NODE: &str = "backend.contact.no_home_node";

    /// Contact refusal `home_insecure_transport`: the home node's address
    /// is plain HTTP on the network, which a card cannot carry.
    pub const CONTACT_HOME_INSECURE: &str = "backend.contact.home_insecure_transport";
    /// Contact refusal `home_profile_stale`: the home-node profile must be
    /// refreshed first.
    pub const CONTACT_HOME_STALE: &str = "backend.contact.home_profile_stale";
    /// A refused card export (`invalid_card` or a code not known yet).
    pub const CONTACT_EXPORT_FAILED: &str = "backend.contact.export_failed";

    /// Every key above (each needs a translation).
    pub const ALL: [&str; 51] = [
        NO_ROUTE,
        NO_GROUP_LINK,
        NO_MEMBER_ROUTE,
        NOT_A_MEMBER_DEVICE,
        NO_COMMITMENT,
        NO_RELAY,
        NO_SELF,
        NO_DIRECTION,
        ATTACHMENT_TOO_LARGE,
        FILE,
        GROUP_ROUTES_UNRESOLVED,
        LOCAL_NETWORK,
        LOCAL_NETWORK_MAYBE,
        PARTIAL,
        START_NO_IDENTITY,
        START_NO_TARGET,
        START_UNVERIFIED_KEY,
        SURFACE_ACTION_REJECTED,
        SURFACE_REFUSED,
        SURFACE_STALE_VERSION,
        SURFACE_STALE_HASH,
        SURFACE_NOT_AUTHOR,
        SURFACE_NOT_OUT_OF_SYNC,
        SURFACE_RATE_LIMITED,
        SURFACE_TOO_LARGE,
        SURFACE_NOT_FOUND,
        SURFACE_OUT_OF_SYNC,
        SURFACE_NOT_ACTION_EXECUTOR,
        SURFACE_ALREADY_USED,
        SURFACE_CLOSED,
        SURFACE_NOT_PERMITTED,
        SURFACE_INVALID_REQUEST,
        GROUP_MEMBER_ROUTE,
        CONTACT_UNKNOWN_PRINCIPAL,
        CONTACT_INVALID_CARD,
        CONTACT_CARD_SIGNATURE_INVALID,
        CONTACT_CARD_EXPIRED,
        CONTACT_SELF,
        CONTACT_ALREADY,
        CONTACT_ROOT_CONFLICT,
        CONTACT_MANIFEST_INVALID,
        CONTACT_AMBIGUOUS,
        CONTACT_RATE_LIMITED,
        PRINCIPAL_ID_TAKEN,
        DEVICE_ID_TAKEN,
        CONTACT_UNAVAILABLE,
        CONTACT_ROOT_REPLACEMENT,
        CONTACT_NO_HOME_NODE,
        CONTACT_HOME_INSECURE,
        CONTACT_HOME_STALE,
        CONTACT_EXPORT_FAILED,
    ];
}

/// `key` with `args`, in the form the UI reads.
pub fn keyed(key: &str, args: &[(&str, &str)]) -> String {
    if args.is_empty() {
        return key.to_owned();
    }
    let map: Map<String, Value> =
        args.iter().map(|(name, value)| ((*name).to_owned(), Value::from(*value))).collect();
    format!("{key} {}", Value::Object(map))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_carry_their_arguments_as_json() {
        assert_eq!(keyed(key::NO_ROUTE, &[]), key::NO_ROUTE);
        let message = keyed(key::PARTIAL, &[("done", "1"), ("total", "2")]);
        let (name, args) = message.split_once(' ').expect("arguments");
        assert_eq!(name, key::PARTIAL);
        let args: Value = serde_json::from_str(args).expect("json");
        assert_eq!(args, serde_json::json!({ "done": "1", "total": "2" }));
    }
}
