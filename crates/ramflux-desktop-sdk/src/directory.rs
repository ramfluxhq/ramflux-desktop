// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! What the daemon's client directories list that requests need: this
//! account's own ids, conversation routes, identities with their verified
//! devices, and object directions.
//!
//! The translator fills it from `account.status`, `device.list`,
//! `conversation.directory`, `identity.directory`, `group.list` and
//! `object.directory`; the planner reads it. Every field is what a directory
//! row said; a field the daemon reported as null stays `None`, and nothing
//! is derived from another id (a group id is never a conversation id, a
//! sender is never a participant).

use std::collections::BTreeMap;

use ramflux_desktop_core::{ContextId, ContextKind, Direction, ObjectId};
use serde_json::Value;

/// A non-empty string field, or `None`.
pub(crate) fn field(row: &Value, name: &str) -> Option<String> {
    row.get(name).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned)
}

/// The directories, as last listed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Directory {
    /// This account's principal id.
    pub self_principal: Option<String>,
    /// This account's principal commitment.
    pub self_commitment: Option<String>,
    /// The relay endpoint of the account's node (`quic://` + its relay
    /// address, as the daemon names it), for attachments.
    pub relay_endpoint: Option<String>,
    /// Conversations by id.
    pub conversations: BTreeMap<ContextId, Conversation>,
    /// Identities by principal id.
    pub identities: BTreeMap<String, Identity>,
    /// Groups by group id.
    pub groups: BTreeMap<String, Group>,
    /// Objects by id.
    pub objects: BTreeMap<ObjectId, Direction>,
}

/// One conversation row.
#[derive(Debug, Clone, PartialEq)]
pub struct Conversation {
    /// Kind; `None` for a legacy row without a classification.
    pub kind: Option<ContextKind>,
    /// The group this conversation belongs to.
    pub group_id: Option<String>,
    /// The route of a direct conversation, when known and active.
    pub direct_route: Option<DirectRoute>,
}

/// The route of a direct conversation.
#[derive(Debug, Clone, PartialEq)]
pub struct DirectRoute {
    /// The recipient's principal commitment.
    pub commitment: Option<String>,
    /// Recipient devices; the first is the selected one.
    pub device_ids: Vec<String>,
    /// Delivery target of the selected device.
    pub target: Option<String>,
    /// Federation route, for a peer on another node.
    pub federation: Option<Value>,
}

/// One group row of `group.list`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Group {
    /// Group epoch.
    pub epoch: Option<u64>,
    /// Roles by member device id.
    pub roles: BTreeMap<String, String>,
    /// Every current member, resolved or not.
    pub members: Vec<MemberRoute>,
}

/// A group member's route.
#[derive(Debug, Clone, PartialEq)]
pub struct MemberRoute {
    /// Member device id.
    pub member_id: String,
    /// The member's principal, when the daemon knows it.
    pub principal_id: Option<String>,
    /// Whether commitment, signing key and target are all known.
    pub delivery_target_known: bool,
}

/// One identity row.
#[derive(Debug, Clone, PartialEq)]
pub struct Identity {
    /// Principal commitment (the contact identity commitment).
    pub commitment: Option<String>,
    /// This account's own identity.
    pub is_self: bool,
    /// Verified devices, in the daemon's order.
    pub devices: Vec<IdentityDevice>,
}

/// A verified device of an identity.
#[derive(Debug, Clone, PartialEq)]
pub struct IdentityDevice {
    /// Device id.
    pub device_id: String,
    /// Verified signing public key.
    pub signing_key: Option<String>,
    /// Delivery target.
    pub target: Option<String>,
}

impl IdentityDevice {
    /// Whether the device can be addressed (key and target known).
    pub fn routable(&self) -> bool {
        self.signing_key.is_some() && self.target.is_some()
    }
}

impl Conversation {
    /// Reads a `conversation.directory` row.
    pub fn from_row(row: &Value) -> Self {
        let kind = match row["kind"].as_str() {
            Some("direct") => Some(ContextKind::Direct),
            Some("group") => Some(ContextKind::Group),
            _ => None,
        };
        let route = &row["direct_route"];
        let direct_route = route.is_object().then(|| DirectRoute {
            commitment: field(route, "recipient_principal_commitment"),
            device_ids: route["recipient_device_ids"]
                .as_array()
                .map(|ids| ids.iter().filter_map(Value::as_str).map(str::to_owned).collect())
                .unwrap_or_default(),
            target: field(route, "target_delivery_id"),
            federation: route.get("federation").filter(|f| f.is_object()).cloned(),
        });
        Self { kind, group_id: field(row, "group_id"), direct_route }
    }
}

impl MemberRoute {
    /// Reads one `member_routes` entry.
    pub fn from_row(row: &Value) -> Option<Self> {
        Some(Self {
            member_id: field(row, "member_id")?,
            principal_id: field(row, "principal_id"),
            delivery_target_known: row["delivery_target_known"].as_bool().unwrap_or(false),
        })
    }

    /// The routes of a row's `member_routes`.
    pub fn list(row: &Value) -> Vec<Self> {
        row["member_routes"]
            .as_array()
            .map_or_else(Vec::new, |routes| routes.iter().filter_map(Self::from_row).collect())
    }
}

impl Group {
    /// Reads a `group.list` row.
    pub fn from_row(row: &Value) -> Self {
        Self {
            epoch: row["group_epoch"].as_u64(),
            roles: row["roles"]
                .as_object()
                .map(|roles| {
                    roles
                        .iter()
                        .filter_map(|(m, r)| r.as_str().map(|r| (m.clone(), r.to_owned())))
                        .collect()
                })
                .unwrap_or_default(),
            members: MemberRoute::list(row),
        }
    }
}

impl Identity {
    /// Reads an `identity.directory` row.
    pub fn from_row(row: &Value) -> Self {
        let devices = row["devices"]
            .as_array()
            .map(|devices| {
                devices
                    .iter()
                    .filter_map(|d| {
                        Some(IdentityDevice {
                            device_id: field(d, "device_id").or_else(|| field(d, "id"))?,
                            signing_key: field(d, "signing_public_key"),
                            target: field(d, "target_delivery_id"),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self {
            commitment: field(row, "contact_identity_commitment")
                .or_else(|| field(row, "principal_commitment")),
            is_self: row["is_self"].as_bool().unwrap_or(false),
            devices,
        }
    }
}

impl Directory {
    /// The principal a device belongs to, from identities and group routes.
    pub fn principal_of_device(&self, device: &str) -> Option<&str> {
        self.identities
            .iter()
            .find(|(_, identity)| identity.devices.iter().any(|d| d.device_id == device))
            .map(|(principal, _)| principal.as_str())
            .or_else(|| {
                self.groups
                    .values()
                    .flat_map(|g| &g.members)
                    .find(|m| m.member_id == device)
                    .and_then(|m| m.principal_id.as_deref())
            })
    }

    /// The group of a conversation, when the daemon linked one.
    pub fn group_of(&self, context: &ContextId) -> Option<&str> {
        self.conversations.get(context).and_then(|c| c.group_id.as_deref())
    }

    /// The member devices of `principal` in `group` (a member whose
    /// principal is unknown is named by its device id).
    pub fn member_devices(&self, group: &str, principal: &str) -> Vec<String> {
        self.groups
            .get(group)
            .map(|g| {
                g.members
                    .iter()
                    .filter(|m| m.principal_id.as_deref().unwrap_or(&m.member_id) == principal)
                    .map(|m| m.member_id.clone())
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn rows_keep_unknown_fields_unknown() {
        let legacy =
            Conversation::from_row(&json!({ "id": "c", "kind": null, "direct_route": null }));
        assert_eq!(legacy, Conversation { kind: None, group_id: None, direct_route: None });
        let dm = Conversation::from_row(&json!({
            "kind": "direct",
            "direct_route": { "recipient_principal_commitment": "cm", "recipient_device_ids": ["d1", "d2"],
                              "target_delivery_id": null },
        }));
        let route = dm.direct_route.expect("route");
        assert_eq!(route.device_ids, ["d1", "d2"]);
        assert_eq!(route.target, None);
        assert_eq!(route.federation, None);
        let identity = Identity::from_row(&json!({
            "principal_id": "p", "contact_identity_commitment": "cp", "is_self": false,
            "devices": [{ "device_id": "d1", "signing_public_key": "k", "target_delivery_id": null }],
        }));
        assert_eq!(identity.commitment.as_deref(), Some("cp"));
        assert!(!identity.devices[0].routable());
    }
}
