// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Desktop contract method names used in [`super::Effect::Call`] and
//! [`super::Effect::SecretCall`].
//!
//! These are contract names, not daemon RPC names; an SDK-backed source maps
//! them to daemon calls.

/// Send a text message.
pub const SEND_MESSAGE: &str = "message.send";
/// Reply to a message with a text that quotes it.
pub const MESSAGE_REPLY: &str = "message.reply";
/// Replace the text of one of this account's messages.
pub const MESSAGE_EDIT: &str = "message.edit";
/// Delete a message for everyone in the conversation.
pub const MESSAGE_DELETE: &str = "message.delete";
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
/// Cancel a transfer that is on its way.
pub const OBJECT_CANCEL: &str = "object.transfer.cancel";
/// Invoke a surface action.
pub const SURFACE_ACTION: &str = "surface.action";
/// Ask the author of an out-of-sync surface for its current state.
pub const SURFACE_REQUEST_RESYNC: &str = "surface.request_resync";
/// Open a card's runtime session on a surface.
pub const CARD_SESSION_OPEN: &str = "card.session.open";
/// Read an open card again (after `surface.changed`).
pub const CARD_SESSION_UPDATE: &str = "card.session.update";
/// Present an open card in another locale or theme.
pub const CARD_SESSION_PRESENT: &str = "card.session.present";
/// A host event on one node of a card.
pub const CARD_SESSION_EVENT: &str = "card.session.event";
/// The host's answer to a card slot (link sheet, approval).
pub const CARD_SESSION_SLOT: &str = "card.session.slot";
/// Submit a card (the only networked card call).
pub const CARD_SESSION_SUBMIT: &str = "card.session.submit";
/// Suspend a card's session (its tree and drafts stay).
pub const CARD_SESSION_SUSPEND: &str = "card.session.suspend";
/// Resume a suspended card under a fresh session.
pub const CARD_SESSION_RESUME: &str = "card.session.resume";
/// Release a card's session.
pub const CARD_SESSION_DISPOSE: &str = "card.session.dispose";
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
/// Read a contact's identity state (`contact.verification.status`).
pub const IDENTITY_STATUS: &str = "identity.status";
/// Accept a contact's changed identity (`contact.trust.accept_change`).
pub const IDENTITY_ACCEPT_CHANGE: &str = "identity.accept_change";
/// Read a direct conversation's security (`conversation.security`).
pub const CONVERSATION_SECURITY: &str = "conversation.security";
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
/// Change a member's role.
pub const GROUP_SET_ROLE: &str = "group.set_role";
/// Leave a group (this account removes itself).
pub const GROUP_LEAVE: &str = "group.leave";
/// Accept an invitation to a group.
pub const GROUP_INVITATION_ACCEPT: &str = "group.invitation.accept";
/// Decline an invitation to a group.
pub const GROUP_INVITATION_DECLINE: &str = "group.invitation.decline";
/// List this account's MLS groups (wire `group.list`).
pub const MLS_GROUP_LIST: &str = "mls_group.list";
/// Read one MLS group (wire `group.info`).
pub const MLS_GROUP_INFO: &str = "mls_group.info";
/// Create an MLS group (wire `group.create`).
pub const MLS_GROUP_CREATE: &str = "mls_group.create";
/// Create an invite of an MLS group (wire `group.invite.create`).
pub const MLS_GROUP_INVITE_CREATE: &str = "mls_group.invite.create";
/// List the invites this account created (wire `group.invite.list`).
pub const MLS_GROUP_INVITE_LIST: &str = "mls_group.invite.list";
/// Revoke an invite (wire `group.invite.revoke`).
pub const MLS_GROUP_INVITE_REVOKE: &str = "mls_group.invite.revoke";
/// Join a group with a link (wire `group.join`).
pub const MLS_GROUP_JOIN: &str = "mls_group.join";
/// Read the join requests the SDK holds (wire `group.join.pending`).
pub const MLS_GROUP_JOIN_PENDING: &str = "mls_group.join.pending";
/// Read a page of members (wire `group.members`).
pub const MLS_GROUP_MEMBERS: &str = "mls_group.members";
/// Read a page of history (wire `group.history`).
pub const MLS_GROUP_HISTORY: &str = "mls_group.history";
/// Send a text post (wire `group.send`).
pub const MLS_GROUP_SEND: &str = "mls_group.send";
/// Fetch from the group host now (wire `group.refresh`).
pub const MLS_GROUP_REFRESH: &str = "mls_group.refresh";
/// Move the read position (wire `group.mark_read`).
pub const MLS_GROUP_MARK_READ: &str = "mls_group.mark_read";
/// Delete an own post for everyone (wire `group.message.delete`).
pub const MLS_GROUP_MESSAGE_DELETE: &str = "mls_group.message.delete";
/// Leave a group (wire `group.leave`).
pub const MLS_GROUP_LEAVE: &str = "mls_group.leave";

/// Read a page of join requests (wire `group.requests.list`).
pub const MLS_GROUP_REQUESTS_LIST: &str = "mls_group.requests.list";
/// Approve a join request (wire `group.requests.approve`).
pub const MLS_GROUP_REQUESTS_APPROVE: &str = "mls_group.requests.approve";
/// Decline a join request (wire `group.requests.decline`).
pub const MLS_GROUP_REQUESTS_DECLINE: &str = "mls_group.requests.decline";
/// Change a member's role (wire `group.role.set`).
pub const MLS_GROUP_ROLE_SET: &str = "mls_group.role.set";
/// Remove a member (wire `group.member.remove`).
pub const MLS_GROUP_MEMBER_REMOVE: &str = "mls_group.member.remove";
/// Ban a member (wire `group.member.ban`).
pub const MLS_GROUP_MEMBER_BAN: &str = "mls_group.member.ban";
/// Change the name or the approval mode (wire `group.settings.set`).
pub const MLS_GROUP_SETTINGS_SET: &str = "mls_group.settings.set";
/// Set this device's notification mode for a group (wire `group.push.set`).
pub const MLS_GROUP_PUSH_SET: &str = "mls_group.push.set";
/// Skip the catch-up (wire `group.catch_up`, always with `skip: true`).
pub const MLS_GROUP_CATCH_UP: &str = "mls_group.catch_up";
/// Undo the head commit (wire `group.void`).
pub const MLS_GROUP_VOID: &str = "mls_group.void";

/// Whether `method` is a contract method of the MLS groups.
pub fn is_mls_group(method: &str) -> bool {
    method.starts_with("mls_group.")
}
/// Add a contact by the username they claimed (`@name`).
pub const USERNAME_CONTACT_ADD: &str = "username.contact.add";

/// Look up a username (`@name`) in the username directory.
pub const USERNAME_RESOLVE: &str = "username.resolve";

/// Claim or change this account's username (needs the recovery phrase).
pub const USERNAME_CLAIM: &str = "username.claim";

/// Read this account's username status (`username.status`).
pub const USERNAME_STATUS: &str = "username.status";
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
/// The desktop's read of an image slot's bytes through the object API
/// (`object.fetch`, `object.get.begin/read/finish`).
pub const CARD_IMAGE_LOAD: &str = "card.image.load";
/// The desktop's composite card approval: `grant.approve` or `grant.deny`,
/// then `task.resolve` with the body core built (carries a secret).
pub const CARD_APPROVAL_FINISH: &str = "card.approval.finish";
/// Unlock an account (carries a secret).
pub const ACCOUNT_UNLOCK: &str = "account.unlock";
/// Lock an account.
pub const ACCOUNT_LOCK: &str = "account.lock";
/// Remove an account from this device (secret body: the confirmation).
pub const ACCOUNT_REMOVE_LOCAL: &str = "account.remove_local";
/// Read an account's app lock (the policy and whether it is locked now).
pub const APP_LOCK_STATE: &str = "account.app_lock.state";
/// Turn the app lock on or off and set its timeout (the account must be open).
pub const APP_LOCK_CONFIGURE: &str = "account.app_lock.configure";
/// Answer the account code sheet of an app lock change (carries a secret;
/// handled by the source, never sent to the daemon).
pub const APP_LOCK_CODE_ANSWER: &str = "app_lock.code.answer";
/// Report that the app went to the background or came back to the front.
pub const APP_LOCK_LIFECYCLE: &str = "account.app_lock.lifecycle";
/// Rotate an account passphrase (carries secrets).
pub const ACCOUNT_ROTATE_PASSPHRASE: &str = "account.passphrase.rotate";
/// Start a full backup of an account to a file (carries a secret).
pub const BACKUP_CREATE: &str = "backup.create";
/// Start reading and verifying a full backup file (carries a secret).
pub const RESTORE_VERIFY: &str = "restore.verify";
/// Install a verified backup (merging with an account already here).
pub const RESTORE_INSTALL: &str = "restore.install";
/// Cancel a running backup, or a restore before it installs.
pub const BACKUP_CANCEL: &str = "backup.cancel";
/// Start creating an account: takes no seeds, answers `{ceremony: pending,
/// ceremony_id}`.
pub const CUSTODY_BEGIN: &str = "account.create";
/// Hand the recovery phrase out, once.
pub const CUSTODY_SHOW_ONCE: &str = "account.create.show_once";
/// The phrase was erased from the screen: choose the three words to ask.
pub const CUSTODY_HIDDEN_CHALLENGE: &str = "account.create.hidden_challenge";
/// Check the three words and create the account (carries secrets).
pub const CUSTODY_CONFIRM: &str = "account.create.confirm";
/// Cancel the ceremony and erase the phrase.
pub const CUSTODY_CANCEL: &str = "account.create.cancel";
/// List the agents of the account (one page; the journal cursor follows).
pub const AGENT_LIST: &str = "agent.list";
/// Read the changed agent records since a journal cursor.
pub const AGENT_WATCH: &str = "agent.watch";
/// Read one agent record.
pub const AGENT_GET: &str = "agent.get";
/// Ask for a creation request (answers the custody status).
pub const AGENT_CREATE: &str = "agent.create";
/// Read the custody status of a creation request (empty id: the next one
/// that waits for this device).
pub const AGENT_OPERATION_GET: &str = "agent.operation.get";
/// Read the PRX catalog of models, tools and reserved names.
pub const AGENT_CATALOG: &str = "agent.catalog";
/// Change an agent's configuration (carries the revision it was read at).
pub const AGENT_UPDATE: &str = "agent.update";
/// Pause an agent (carries the revision it was read at).
pub const AGENT_PAUSE: &str = "agent.pause";
/// Resume an agent (carries the revision it was read at).
pub const AGENT_RESUME: &str = "agent.resume";
/// Edit the persona of an agent.
pub const AGENT_PERSONA_EDIT: &str = "agent.persona.edit";
/// Set the access of an agent (carries the base it was read with).
pub const AGENT_ACCESS_SET: &str = "agent.access.set";
/// Delete an agent in state U.
pub const AGENT_DELETE: &str = "agent.delete";
/// Pause a PRX job of an agent (cron, xin_task or xin_goal).
pub const AGENT_JOB_PAUSE: &str = "agent.job.pause";
/// Resume a PRX job of an agent.
pub const AGENT_JOB_RESUME: &str = "agent.job.resume";
/// Remove a PRX job of an agent.
pub const AGENT_JOB_REMOVE: &str = "agent.job.remove";
/// List the PRX jobs of an agent.
pub const AGENT_JOB_LIST: &str = "agent.job.list";
/// List the approvals of agents waiting for an answer.
pub const AGENT_APPROVAL_LIST: &str = "agent.approval.list";
/// Approve or deny one approval of an agent.
pub const AGENT_APPROVAL_DECIDE: &str = "agent.approval.decide";
/// Begin recovering an account from its phrase (user-started only).
pub const RECOVER_BEGIN: &str = "account.recover.begin";
/// Check the fingerprint, the phrase and the passphrase and recover (carries
/// secrets).
pub const RECOVER_CONFIRM: &str = "account.recover.confirm";
/// Cancel the recovery.
pub const RECOVER_CANCEL: &str = "account.recover.cancel";
/// Revoke a device.
pub const DEVICE_REVOKE: &str = "device.revoke";
/// Export own-device sync data.
pub const DEVICE_SYNC_EXPORT: &str = "device.sync.export";
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
