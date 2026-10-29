// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Desktop contract method names used in [`super::Effect::Call`] and
//! [`super::Effect::SecretCall`].
//!
//! These are contract names, not daemon RPC names; an SDK-backed source maps
//! them to daemon calls.

/// Send a text message.
pub const SEND_MESSAGE: &str = "message.send";
/// Start a direct conversation with a contact by sending its first message.
pub const CONVERSATION_START: &str = "conversation.start";
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
/// Ask the author of an out-of-sync surface for its current state.
pub const SURFACE_REQUEST_RESYNC: &str = "surface.request_resync";
/// Report what a renderer could not show (to the surface producer).
pub const RENDERER_ERROR: &str = "surface.renderer_error";
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
/// Record the requester's decision on a task step (sent by a source after
/// the grant or surface decision it refers to; never an intent of its own).
pub const TASK_RESOLVE: &str = "task.resolve";
/// Pin a context.
pub const CONTEXT_PIN: &str = "context.pin";
/// Unpin a context.
pub const CONTEXT_UNPIN: &str = "context.unpin";
/// Mute or unmute a context.
pub const CONTEXT_MUTE: &str = "conversation.mute";
/// Set the disappearing-message timer.
pub const CONTEXT_DISAPPEARING: &str = "conversation.disappearing.set";
/// Add a contact by the id they shared.
pub const CONTACT_ADD: &str = "contact.add";
/// Add a contact from the contact card they shared.
pub const CONTACT_CARD_IMPORT: &str = "contact.card.import";
/// Export this account's own contact card.
pub const CONTACT_CARD_EXPORT: &str = "contact.card.export";
/// Send a contact request to the contact's node.
pub const CONTACT_REQUEST: &str = "contact.request";
/// Accept a contact request.
pub const CONTACT_ACCEPT: &str = "contact.accept";
/// Reject a contact request.
pub const CONTACT_REJECT: &str = "contact.reject";
/// Block a contact.
pub const CONTACT_BLOCK: &str = "contact.block";
/// Lift a block.
pub const CONTACT_UNBLOCK: &str = "contact.unblock";
/// Remove a contact.
pub const CONTACT_REMOVE: &str = "contact.remove";
/// Accept an incoming contact request (its held messages are delivered).
pub const CONTACT_REQUEST_ACCEPT: &str = "contact.request.accept";
/// Decline an incoming contact request (its held messages are dropped).
pub const CONTACT_REQUEST_DECLINE: &str = "contact.request.decline";
/// Block the requester of an incoming contact request.
pub const CONTACT_REQUEST_BLOCK: &str = "contact.request.block";
/// Set how contact requests are answered (`manual` or `auto_accept`).
pub const CONTACT_REQUESTS_POLICY: &str = "contact.requests.policy.set";
/// Verify a safety number.
pub const SAFETY_VERIFY: &str = "safety.verify";
/// Create a group with its conversation and first members.
pub const GROUP_CREATE: &str = "group.create";
/// Rename a group conversation (the name this account keeps for it).
pub const CONTEXT_RENAME: &str = "conversation.rename";
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
/// Place a voice call in a direct conversation (CALL-01 §6 `call.invite`).
pub const CALL_INVITE: &str = "call.invite";
/// Answer a ringing call.
pub const CALL_ANSWER: &str = "call.answer";
/// Decline a ringing call.
pub const CALL_REJECT: &str = "call.reject";
/// Hang up (or cancel) a call.
pub const CALL_HANGUP: &str = "call.hangup";
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
/// Export an encrypted account backup (carries a secret).
pub const ACCOUNT_BACKUP_EXPORT: &str = "account.backup.export";
/// Restore an account from a backup (carries a secret).
pub const ACCOUNT_BACKUP_IMPORT: &str = "account.backup.import";
/// Start a full backup of an account to a file (carries a secret).
pub const BACKUP_CREATE: &str = "backup.create";
/// Start reading and verifying a full backup file (carries a secret).
pub const RESTORE_VERIFY: &str = "restore.verify";
/// Install a verified backup (merging with an account already here).
pub const RESTORE_INSTALL: &str = "restore.install";
/// Cancel a running backup, or a restore before it installs.
pub const BACKUP_CANCEL: &str = "backup.cancel";
/// Start creating an account: takes no seeds, answers `{ceremony: pending,
/// ceremony_id}`. The same string as [`ACCOUNT_CREATE`], told apart by
/// carrying no secret (the one-shot create is gone once a daemon offers this).
pub const CUSTODY_BEGIN: &str = "account.create";
/// Hand the recovery phrase out, once.
pub const CUSTODY_SHOW_ONCE: &str = "account.create.show_once";
/// The phrase was erased from the screen: choose the three words to ask.
pub const CUSTODY_HIDDEN_CHALLENGE: &str = "account.create.hidden_challenge";
/// Check the three words and create the account (carries secrets).
pub const CUSTODY_CONFIRM: &str = "account.create.confirm";
/// Cancel the ceremony and erase the phrase.
pub const CUSTODY_CANCEL: &str = "account.create.cancel";
/// Begin recovering an account from its phrase (user-started only).
pub const RECOVER_BEGIN: &str = "account.recover.begin";
/// Check the fingerprint, the phrase and the passphrase and recover (carries
/// secrets).
pub const RECOVER_CONFIRM: &str = "account.recover.confirm";
/// Cancel the recovery.
pub const RECOVER_CANCEL: &str = "account.recover.cancel";
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

/// Capability key of `method` performed in a context of `kind`, for
/// backends that support a method only for some context kinds (for example
/// `message.send@Direct`).
pub fn in_context(method: &str, kind: crate::model::ContextKind) -> String {
    format!("{method}@{kind:?}")
}
