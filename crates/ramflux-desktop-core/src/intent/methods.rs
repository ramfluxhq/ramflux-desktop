// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Desktop contract method names used in [`super::Effect::Call`] and
//! [`super::Effect::SecretCall`].
//!
//! These are contract names, not daemon RPC names; an SDK-backed source maps
//! them to daemon calls.

/// Send a text message.
pub const SEND_MESSAGE: &str = "message.send";
/// Send an object.
pub const ATTACH_OBJECT: &str = "object.send";
/// Pull (download) an inbound object.
pub const OBJECT_PULL: &str = "object.get";
/// Save a downloaded object to a local path.
pub const OBJECT_SAVE: &str = "object.save";
/// Resume a failed or paused transfer.
pub const OBJECT_RETRY: &str = "object.transfer.resume";
/// Invoke a surface action.
pub const SURFACE_ACTION: &str = "surface.action";
/// Approve a grant.
pub const GRANT_APPROVE: &str = "grant.approve";
/// Deny a grant.
pub const GRANT_DENY: &str = "grant.deny";
/// Revoke an approved grant.
pub const GRANT_REVOKE: &str = "grant.revoke";
/// Revoke a standing approval.
pub const STANDING_REVOKE: &str = "grant.revoke_standing_approval";
/// Reveal masked content.
pub const PRIVACY_REVEAL: &str = "privacy.reveal";
/// Resolve a conflict.
pub const CONFLICT_RESOLVE: &str = "object.resolve_conflict";
/// Start a task.
pub const TASK_START: &str = "task.start";
/// Cancel a task.
pub const TASK_CANCEL: &str = "task.cancel";
/// Pin a context.
pub const CONTEXT_PIN: &str = "context.pin";
/// Unpin a context.
pub const CONTEXT_UNPIN: &str = "context.unpin";
/// Mute or unmute a context.
pub const CONTEXT_MUTE: &str = "conversation.mute";
/// Set the disappearing-message timer.
pub const CONTEXT_DISAPPEARING: &str = "conversation.disappearing.set";
/// Accept a contact request.
pub const CONTACT_ACCEPT: &str = "contact.accept";
/// Reject a contact request.
pub const CONTACT_REJECT: &str = "contact.reject";
/// Verify a safety number.
pub const SAFETY_VERIFY: &str = "safety.verify";
/// Add a group member.
pub const GROUP_ADD_MEMBER: &str = "group.add_member";
/// Remove a group member.
pub const GROUP_REMOVE_MEMBER: &str = "group.remove_member";
/// Kick a group member.
pub const GROUP_KICK: &str = "group.kick";
/// Ban a participant from a group.
pub const GROUP_BAN: &str = "group.ban";
/// Change a member's role.
pub const GROUP_SET_ROLE: &str = "group.set_role";
/// Create a group invitation.
pub const GROUP_CREATE_INVITE: &str = "group.create_invite";
/// Accept a handoff.
pub const HANDOFF_ACCEPT: &str = "handoff.accept";
/// Decline a handoff.
pub const HANDOFF_DECLINE: &str = "handoff.decline";
/// Unlock an account (carries a secret).
pub const ACCOUNT_UNLOCK: &str = "account.unlock";
/// Lock an account.
pub const ACCOUNT_LOCK: &str = "account.lock";
/// Create an account (carries a secret).
pub const ACCOUNT_CREATE: &str = "account.create";
/// Rotate an account passphrase (carries secrets).
pub const ACCOUNT_ROTATE_PASSPHRASE: &str = "account.passphrase.rotate";
/// Export an encrypted account backup.
pub const ACCOUNT_BACKUP_EXPORT: &str = "account.backup.export";
/// Restore an account from a backup (carries a secret).
pub const ACCOUNT_BACKUP_IMPORT: &str = "account.backup.import";
/// Join an existing identity as a new device (carries a secret).
pub const DEVICE_ACTIVATE: &str = "device.activate";
/// Revoke a device.
pub const DEVICE_REVOKE: &str = "device.revoke";
/// Export own-device sync data.
pub const DEVICE_SYNC_EXPORT: &str = "device.sync.export";
/// Import own-device sync data.
pub const DEVICE_SYNC_IMPORT: &str = "device.sync.import";
/// Discover a node from a URL (returns a summary awaiting confirmation).
pub const NODE_ADD: &str = "node.add";
/// Confirm and pin a discovered node.
pub const NODE_CONFIRM: &str = "node.confirm";
/// Probe a node's gateway and relay.
pub const NODE_PROBE: &str = "node.probe";
/// Remove a node profile.
pub const NODE_REMOVE: &str = "node.remove";
/// Make a node the default for new accounts.
pub const NODE_SET_DEFAULT: &str = "node.set_default";
/// Enable or disable a node for new accounts.
pub const NODE_SET_ENABLED: &str = "node.set_enabled";
/// Export a redacted diagnostics bundle.
pub const DIAGNOSTICS_EXPORT: &str = "diagnostics.export";
