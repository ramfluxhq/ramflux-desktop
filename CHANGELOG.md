# Changelog

All notable changes to this project will be documented in this file.

The format is based on Keep a Changelog, and this project adheres to Semantic
Versioning.

## [Unreleased]

### Changed

- MLS groups screens (MLS GUI batch M2, r4), on the scenario mock only (`--source mock`, scenario `s12_mls_groups`, fixtures F2-1 to F2-10): while the app runs on the mock, the Groups route shows the new MLS groups list (virtualized, paged by cursor, with unread counts and the last post) in place of the old directory; on a live daemon nothing changes. New group (name 1 to 64 characters, approval on by default), the invite panel (any member may create invites; presets of 1, 7 and 30 days and 1, 10 and 100 uses; "Copy link" copies on the tap only, and the desktop has no clipboard expiry, so the entry stays until something else is copied), the join sheet (the link is checked locally and passed verbatim; one call per tap, an attempt number, the outcome of an older attempt is recorded but not shown; pending requests are read from `group.join.pending`), the group chat (history in pages of 50 with the floor line, send states sending, queued, delayed and not sent with Retry as the only way to send again, delete for everyone with a confirmation, a read-only banner after leaving, removal or a ban, a refresh action, mentions from the loaded members), the info panel and the member list (role headers, AI marker from the SDK flag, "Member" for a missing name, search that loads the remaining pages). A used-up daily limit takes the composer away until it resets, with a timer that only redraws. The developer panel shows "mock calls" per method. Join links never print: they travel as a secret field of the request and `Debug` of an effect redacts them.
- MLS groups wire plans and translators (MLS GUI batch M2, r4): `plan/mls_group.rs` builds the requests of `group.list`, `group.info`, `group.create`, `group.invite.create`, `group.invite.list`, `group.invite.revoke`, `group.join` (the link only; the attempt number never goes on the wire), `group.join.pending`, `group.members`, `group.history`, `group.send`, `group.refresh`, `group.mark_read`, `group.message.delete` and `group.leave`; `translate/mls_group.rs` reads their replies, error frames and `group.event` frames (`post`, `post_state`, `post_deleted`, `membership`, `settings`, `join_result`, `read`; other kinds are skipped without logging their payload). The bridge refuses every `mls_group.*` method on a live source with the typed unavailable outcome until the pin batch, and `group.event` is not subscribed yet, because five of the wire names exist at the current pin with the old shapes.
- MLS groups on the MLS surface (MLS GUI batch M2, method table r4), model and store: the account keeps the group list, member pages, the invites it created, the join owner (one call per explicit tap, an attempt number, pending requests held in memory only) and bounded timelines (600 posts, the three most recent groups) in memory, projected into an `mls:<group_id>` conversation context. Own posts carry the states sending, queued, delayed, and not sent; a not-sent post is sent again only by the Retry tap, which makes a new send with a new local id. Invite links never print. The AI marker is merged in the data layer and never dropped once the SDK said it. The old `group.*` module is unchanged; nothing calls the new methods on a live daemon (the pin does not change). A post that is queued or delayed and has a post id survives the eviction of its timeline, so a later `failed` still shows the Not sent bubble; a lock through an account update clears the MLS state; the picked mentions live in the account's state (not in the view), are spent only by an accepted send and go with a lock or removal; a post state that arrives before the send reply is applied by the reply; mark-read is sent once per position.
- Direct-chat identity on the MLS surface (MLS GUI batch M1): a contact's identity is `Verified`, `Tofu` (first seen on this device, no badge) or `Changed`, read with `contact.verification.status` and announced by `contact.identity_changed`; a direct conversation's encryption is read with `conversation.security` (and again on `conversation.devices_changed`). A changed identity shows a header banner with Review (no dismiss), a warning glyph on the conversation row, a service line in the timeline and, in the contact profile, an Encryption section. Review opens the identity change sheet, the only place a change is accepted (`contact.trust.accept_change`): one click, on the anchor the sheet displayed; a newer anchor re-renders the sheet and needs a new click; the pending change is cleared on success only if it is still the accepted one. A send refused with `identity_change_pending` stays a failed message and the composer asks to review the change; nothing is sent again by itself. A call to a contact with a waiting change opens the sheet. The new methods and events are driven on the scenario mock (`--source mock`, scenario `s04_safety`, fixtures F1-1 to F1-10); the wire plans and translators are first run against a real daemon when the core pin moves (the pin does not change here).
- Kit component alignment (boards 00B, 00D, B19b): the kit report goes from 14 to 25 of 36 components within tolerance. The security notice is its own component (`status::security_notice`: 18 px tone icon, semibold title, body in `rf-text`, 10 px gap) and also draws the key-change notice and the device-approval warning, which now wraps inside its card; the page header (`page_header::page_header`) no longer carries the generated empty slots; avatars show their initial in semibold (`avatar::avatar`); a bubble's time shares the body line's 24 px box, 1 px lower, with a 14 px receipt icon; design buttons are 80 px for a four-character label (the label measures 1 px wide) and outlined ones take their edge out of the inset; the search glyph is 16 px with the label 6 px after it at 14 px; the info and ellipsis glyphs are 18 and 20 px and the ellipsis button is 32 px; the idle segment label is `rf-secondary`. Bubble width rule: a bubble hugs its text up to `rf-bubble-max` (boards 02, B20); the 360 px of board 00B is the component frame, which the kit capture uses.
- Kit alignment, round 2: group avatars have one rendering (`avatar::avatar`): the initial in the person circle for a named group (boards 02, C02; the generated group style painted the initial in its own fill, so it was invisible in the group profile), the users glyph for an unnamed one (board 00D), and the kit measures that code. Settings, Tasks and Contacts headers use `page_header::page_header` (title style parameter; Settings keeps its 24 px title), dropping the 12 px inset of the generated header.
- The add-contact dialog's Cancel button has its own string key, `dock.contacts.add.cancel`, in every catalogue.
- `rustix` updated to 1.1.6 (RUSTSEC-2026-0486, unsound xattr buffers); the desktop only reads the process user id through it.
- Agent management on the pinned surface: the new-agent wizard picks the model from PRX's catalog (`agent.catalog`; a model whose provider key is not held is shown disabled with "Add the provider key first", and a reserved name is refused), an agent's model, autonomy and prompt are changed with `agent.update` on the revision `agent.get` shows (read as the form opens), and an agent is paused and resumed with `agent.pause` and `agent.resume`. A change made elsewhere ("Changed elsewhere. Review and try again.") reads the agent again; a change PRX holds for confirmation asks for it and is resent once with `confirm_high_risk`. Agent texts (persona name and responsibility, approval tool and preview, job ids) lose every Unicode format character (`Cf`). The `prx` the runtime panel runs must be named `prx` after symbolic links, and its search path entry and every folder above it must be owned by root or the process user and not group- or other-writable; the process user comes from a safe wrapper, not from the owner of `$HOME`. An install runs only the file its confirmation named (otherwise "The runtime program changed since you confirmed. Review it again."). Agent approvals and job actions are refused unless the owner is the active account. Agent creation requests in flight are kept in the settings file and polled again after a restart.
- Quitting waits for the daemon's bounded stop: up to 5 s for the answer of `daemon.stop` (it now answers after its shutdown ran, with `forced` and `overran`, which are logged) and then up to 5 s for the process to exit, before it is signalled.
- Cards on the new card runtime: refusals are read from the typed reason (`card_session_expired`, `card_session_inactive`, `card_session_limit`, `card_limit_exceeded`, `card_session_refused`, `card_session_stale`, `card_session_busy`), never from the message. A card whose session expired or was dropped (the app lock closed, sign-out, removal) keeps its last tree read-only and is opened again on the next interaction; the lock no longer suspends cards and unlocking resumes none. The two resident cards of the daemon are counted once for the whole process across accounts, 24 held cards per account stay; a call the runtime refuses for a limit frees the least recently used card and tries once more, and a second refusal shows "Too many cards are active. Try again in a moment." Every event and slot reply carries its batch (an empty one replaces the host slots only; one missing is a protocol error), a stale event applies its batch and says "This content changed. Check it and try again.", and the `validation` result is applied like an edit. An approval slot's Allow or Deny runs `grant.approve` or `grant.deny` and then `task.resolve` with the body the runtime built, unchanged (never edited, never logged); a typed refusal of the task decision says "This request changed. Review it again." and reads the task again, with no automatic retry. Image slots are read through the object API (`object.fetch`, `object.get.begin/read/finish`), checked against the slot's object and digest, kept in a private file and decoded as PNG or JPEG only (each side at most 4096, at most 4,194,304 pixels, 32 MiB decoded, one decode at a time), drawn in the box of their variant with `fit`, with the description as tooltip and name, and a click opens the full view (Escape closes); anything else shows "Image unavailable". The "daemon lacks the card runtime" probe and `backend.gap.surfaces` are gone.
- Lock page: an account whose code attempts are used up shows "Too many wrong account codes" with two actions, "Recover with recovery phrase" and "Remove from this device". Recovery keeps the account fixed and read-only; a wrong phrase restarts at the fingerprint, an accepted phrase with the wait still running returns to the lock form with its live countdown, a wrong code returns to the exhausted page, and the page lowers only on the daemon's app lock state or event. Removal asks in a dialog with an "I understand" box, sends the locked account's own id as the confirmation (secret body, never logged), and on success drops the account, its media folder and its card sessions and routes to the account list or first run. The dialog and the recovery close, and their inputs clear, when the window loses focus or the lock page's account changes. Recovery refusals the SDK does not own (`wrong_passphrase`, `unlock_retry_later`, `unlock_attempts_exhausted`) are read from the typed reason.
- Core pinned to `089f11657` (core `next`): the manifest-and-contacts key backup methods `account.backup.export` / `account.backup.import` are gone from the daemon, so the desktop drops their flows completely: the entry restore path offers only the full restore (no "Key backup" choice, file path or passphrase inputs), Settings · Account has no "Export account key backup" section, and the intents, plan builders, method constants, deadline entry, strings in every locale, the scenario step and the contract text for them are deleted. Recovery, removal and unlock fixes in the core (RECOVER-PUBPENDING-01, LEGACY-UNLOCK-01) need no client change; an agent approval closed at its deadline arrives as the existing `expired` state. `card.session.resume` at the limit now answers the typed `card_limit_exceeded`, so the untyped invalid-argument fallback is gone and a resume at the limit shows the same limit state as an open; the lock page countdown reads `code_retry_after_secs` from `account.app_lock.state`.
- Core pinned to `01174ece` (core next): recovery and removal of an app-locked account (RECOVER-LOCKED-01), card runtime batch A (typed codes, session drops, approvals, image slots), bounded daemon stop, MLS foundation (no client behaviour). `rf` is built with `--profile ffi-release` (the card runtime is always linked).
- Screenshot programs `conversation_shots`, `task_shots`, `desk_pin_shots` and `desk_pin3_shots` run to completion again: the fake daemon answers the prepared-media path (`attachment.prepare.*`, `conversation.send`), `screenshots` drops its removed conformance set, and the programs no longer wait for task event lines or the kind mark, which the SDK source stopped projecting.
- Dialogs (B11, C01, F02, E05, G24): create group, add contact, compare safety numbers, task details and approve linking a web device are modal dialogs over the dimmed window, drawn by one shared container (`modal`: scrim, card width, padding, title row with close, button row). Tab stays inside the card, and Escape or a click on the scrim closes it. Opening a dialog no longer replaces the conversation behind it. The approval dialog is drawn for a request the settings page is given; Approve stays disabled until the pairing contract exists.
- Board captures: the D33 to D38 recipe fills each wizard step's required fields before pressing Next, so D34 to D38 show their own steps and D38 captures the result line again.

- App lock: turning the lock off, or choosing a longer timeout, asks for the account code in a sheet. The code goes to the daemon's attended prompt on the shared request connection, only while that configure runs, and is zeroized on every path. The sheet closes, its code is cleared and the prompt is refused when the window loses focus, Settings closes, the lock or cover rises, or the account changes; only a weakening configure can be prompted, and the next configure starts only after the previous answer is handled. "Wrong code" is shown only on the typed `wrong_passphrase` reason; a wait after wrong codes is a live countdown with no sheet; exhausted codes route to the recovery-phrase restore on the lock screen. Code refusals no longer appear as failure text.
- Core pinned to `a897c417` (core `next`, #649). The core brings the typed app-lock code refusals (APP-LOCK-REAUTH-02, `2a8b6899`), the account-code prompt for `account.app_lock.configure` on the local bus (APP-LOCK-REAUTH-03, #649), the agent management rows (`e42182d7`) and the username witness registry (`859b5fd3`). The desktop does not use the agent or witness rows yet.

### Removed

- Safety numbers: the compare dialog and every compare, verify and mark-verified control, the stale-verification acknowledgements (new conversation, new group, add to group, call), `safety.verify` with `contact.safety_number` and `contact.verify`, the safety number of contact requests, and the safety locale keys. Boards B11, I31 and the safety part of F14 are retired by the MLS decision (their capture recipe and deviation files are removed).

### Added

- `deny.toml` for `cargo deny`: advisories, bans, licences and sources policy modelled on the core, with the workspace AGPL-3.0-or-later licence allowed and the local SQLCipher exceptions of the core. Advisories outside this change are not yet ignored.
- Usernames (G08): Settings > Account shows the claimed `@name` and claims or
  changes it. The account's recovery phrase authorizes the change; it is held
  in a secret for the one call, never serialized, logged or stored, and the
  claim is authorized in process against the computer's username centre
  (bundled configuration file, `RAMFLUX_USERNAME_CENTER_*` as an override).
  Contacts > Add looks a typed `@name` up first and shows what was found
  (display name and bio when published), then "Send contact request" adds the
  contact by username. A directory that cannot be reached, or a computer with
  no username centre, shows a typed state; messaging is unaffected. A claim
  the directory has not settled within the wait shows "Still publishing" with
  a Refresh action (asked for that claim's own operation, ignoring repeats for
  two seconds), and settles when the directory reports it committed or
  aborted. Typed phrases and passphrases are cleared at once when the account
  locks or changes, the section changes, or settings open or close; claim and
  lookup state is kept per account, and answers that arrive after a lock are
  dropped.

### Security

- G10b audit round 2: a card image read always sends `object.get.finish` after a successful `object.get.begin` (also on an error or a cancelled task), so the daemon's plaintext spool never stays open; removing an account erases its pins, stream cursors, name changes and agent creations from the desktop settings file; the install confirmation is disabled when no `prx` file was found and the host refuses an install that names none; removing a locked account is sent only when the lock page's account is the dialog's. Smaller: job ids are not format-stripped, a failed card image write removes its `.part` file, and a card the runtime would not open is retried three times, then only on interaction.
- Agent cards (G10 review round 2 and security audit): a tap is rebased onto
  the desktop's own replies only when the signed version and the pressed
  node's drawn content (attributes, label, action) are exactly what the user
  saw; otherwise it is refused with the "card changed" notice. Text drafts
  merge only with a draft of the same field at the tail of a card's queue.
  Every host event the desktop drops (behind a refusal, a stale card, a
  resume on a new version, a failed resume) leaves the visible notice; events
  waiting behind a call keep the version they are valid on when the card is
  suspended. An event call frees its card whatever its reply, so an `edited`
  reply without a batch no longer holds the card. An approval draws only the
  facts the SDK verified (requester, action, scope, expiry, risk) inside a
  host frame no card can draw (heavy border, shield heading strip; the
  warning glyph is no longer a card icon); an approval without verified
  facts shows "not waiting for your decision" and no buttons, and cannot be
  answered. A link with any character outside printable ASCII is refused, not
  cleaned for display; the link sheet is dropped on suspend and resume, and
  the host is checked again right before the browser opens. Card trees,
  batches, slots, tokens, mutations and the seen node content print redacted
  in `Debug`, and the parsed secret of an event or slot call is wiped.
  `scripts/package-macos.sh` refuses a release binary whose dependency graph
  holds the card runtime crate or QuickJS, or that carries card gallery or
  QuickJS symbols.
- Agent cards (G10 review round 1): every host event and slot answer carries
  the tree state the user saw and is refused when the held tree has moved on
  (`card_changed`), except that a text draft may be re-sent on the desktop's
  own chain of replies. Tree text is drawn with bidirectional controls
  removed. A link sheet shows the host on its own line and opens only an
  https link the runtime answered `opened` for, and a modal sheet names its
  author with the AI marker.
- Agent management (G19): the PRX provider key field is now masked and kept
  out of the clipboard, and the key is trimmed before use. An agent's
  approval names the agent (from the owner's roster), the tool and the
  preview as plain clipped text marked as the agent's own words, reads its
  RFC 3339 expiry and offers no answer once expired; the answer is refused
  on an account mismatch. Agent mutations no longer carry a frame operation
  id, which the daemon refuses (`CustodySetupInvalid`). Jobs are touched
  only for a live agent of the owner. `prx` is resolved once to an absolute
  path (relative search path entries, and folders or files others can write
  or another user owns, are refused) and the install confirmation shows it.
- Agent cards: a host event that finds its card suspended is kept until the
  card has resumed and is then sent on the resumed tree (a text draft is
  rebased onto it; a tap only when the signed version is unchanged), instead
  of being consumed by the resume.

### Added

- Card sessions: a card's text drafts and taps queue behind the event call in
  flight on the card; a refused open is not asked again until the surface
  changes; a card's sessions are released when its account is removed or
  reloaded; a refused dispose can be asked again. The `card_gallery.*` strings
  are compiled in only with `--features card-gallery`.
- Card gallery and conformance (test builds only, `--features card-gallery`):
  `--card-gallery` lists every host fixture and scenario and draws each through
  the card renderer, live against the standalone card runtime; the conformance
  (checks A to C) runs from the window, and `--card-gallery-exit` runs it once
  and exits. The gallery is never part of the default or the release build.
- Fixture sync: `scripts/sync-card-fixtures.sh` copies the core's card host
  fixtures and the bundled trust package at the pinned revision into
  `card-fixtures/` with a BLAKE3 `MANIFEST.json`; copies that drift are refused.
- Demo sessions draw cards from the fixture trees for the demo surfaces that
  carry one; other demo surfaces keep their fallback text.

- Cards are drawn from the card runtime's native tree (`ramflux.card.tree.v1`,
  all 20 kinds) through the `card.session.*` bus methods. The store holds each
  card's tree and applies its mutation batches atomically (a refused batch
  leaves the tree as it was). Text is plain; images are never fetched and show
  "Image unavailable". Interactions: typed input, buttons (drafts travel with
  the press), tabs, modal sheets, link confirmation (the browser opens only
  after the runtime answers `opened`) and approvals (the desktop shows "Waiting
  for confirmation" after its verdict; it does not decide the task step yet).
  Lifecycle states (generating, submitting, consumed, closed, updated, out of
  sync, stale) follow the runtime. Sessions stay within the runtime's limits
  (two resumed, twenty-four open, least recently used first) and are suspended
  when the window is away for a minute or the app locks.
- Card runtime groundwork: the SDK builds the `card.session.*` bus requests and
  types their replies; event, slot and submit bodies travel as secret fields;
  a daemon without the runtime is reported as `backend.gap.surfaces`.

- App lock (Settings › Account › App lock, off by default): when on, the
  app asks for the account code after the chosen time in the background
  (immediately, 1 minute, 5 minutes, 15 minutes or 1 hour). The lock is the
  core's (`account.app_lock.configure`, `.state` and `.lifecycle`); while it
  holds the account closed, a lock page asks for the account code and shows
  nothing else, and the content stays covered until the lock state is known
  after the window comes back. Notices show no sender and no text. Unlock is by account code only:
  biometric unlock waits for the core.
- Named agents on the desktop (G19, AGENT-IDENTITY-01): Settings › Agents
  lists the agents the account owns, grouped as a main agent with its
  sub-agents (the sub-agent action is disabled with its reason), with
  loading, offline and failed states. The new-agent wizard collects the
  persona (name, avatar preset, what it is for), the provider and model,
  tools and limits, who may reach the agent, and reviews them. Create asks
  the account's phone to approve; the result follows the request (pending
  approval with a countdown, confirming, created, denied, expired, cancelled
  or failed with its reason).
- Agent management: edit the persona, change who may reach an agent (with
  the record's base, so a stale record is refused), remove an agent, and
  pause, resume or remove its PRX scheduled jobs.
- Agent approvals (in a conversation) are listed in the approvals dock, where
  they can be allowed or denied.
- The agents' PRX jobs are read when their panel opens, and the owned daemon
  restarts with the runtime's state directory after an install or a restart,
  so it adopts the runtime.
- The PRX runtime panel: status, install (the provider key is typed into a
  masked field, used once and never shown again), restart, stop and
  uninstall. Features the core or PRX do not offer yet show a typed
  unavailable reason instead of a control that does nothing.
- Message actions in direct and group conversations: Reply (the composer
  quotes the message; the bubble shows the quote), Edit (with the revision
  the core expects; the message is marked edited), Delete for everyone
  (confirmed on the strip) and Copy, from the message's actions button, its
  context menu or the keyboard. They use the core's conversation mutations
  (`conversation.reply`, `.edit`, `.delete`).
- Read receipts: the conversation shown is marked read through its newest
  message (`message.receipt.read`; the core routes the receipt to the
  author's device), and own messages show Read when the peer read them.
- Delete for me in a message's menu: the message leaves this device's
  conversation only (`conversation.delete` with scope `local`), with its own
  confirmation.
- A message that the peer edits or deletes updates on screen without
  reopening the conversation; a reply quote jumps to and highlights the
  message it quotes.
- Conversation names from the core (`peer_display_name` on direct rows).
- Leave group (the conversation's menu, confirmed) and group invitations
  from people who are not contacts: listed in the contacts dock with Join
  and Decline (`group.invitations.list`, `group.invitation.accept` and
  `.decline`, the `group.invitation` event).
- Usernames: Settings › Account shows the username the directory reports
  or why it is unavailable; Add contact accepts `@name`
  (`username.contact.add`).

### Changed

- Core pinned to 7f534b2f (core `next`, PRX pin 3a5a6084). The core adds the
  app-lock re-authentication with the account code, central push routes, and
  FCM parity.
- Task-step decisions on a surface are still answered through the surface's
  own actions (`surface.action`, the core's surface store and `surface.get`
  read model). This is a named interim: the card approval flow (`task.resolve`
  with its decision id and revision) replaces it with G12. Approval verdicts
  from a card stop at "Waiting for confirmation".
- Agents no longer use the AI participant credential export or the
  headless-AI account ceremony; the wizard's unlock-free flow is replaced by
  the desktop-started creation above. The bus methods are the
  AGENT-IDENTITY-01 `agent.*` set, and the daemon owned by the desktop
  adopts the PRX runtime after an install or restart (`--prx-state-dir`).

- Core pinned to 69fca252 (core `next`, PRX pin 6e39b396). The username directory is
  provisioned by `rf daemon start` from its environment (`RAMFLUX_USERNAME_CENTER_*`,
  `RAMFLUX_USERNAME_TRUST_GENERATION`); the desktop passes its environment on.
- A direct conversation's header no longer shows a member count.
- Adding a contact card confirms ("Contact card added.") also when the card
  named a contact that was already known.
- The AI marker follows the core's signed classification
  (`verified_classification`: person, AI agent, unknown); an unproven claim
  no longer draws a party as an agent.

### Fixed

- Card sessions: a resumed card is marked resumed in the pool (it was asked to
  resume again on every tap); a live window dispatches card intents through
  the account session's queue; every dispose an account sends before its
  session goes away reaches the daemon; a lost connection or a daemon restart
  forgets the held cards so they open again; nothing is resumed, re-read,
  submitted or sent while the app lock is closed; a submit whose answer is
  unknown keeps the button consumed and offers "Try again" instead of a
  refusal; a desktop-side refusal never releases a consumed button. The demo
  card trees load again (their batches named the tree schema).
- Card choice and slider inputs send the shapes the runtime accepts: a choice
  is a list of option values for every variant, and a slider value is an
  integer within its range.
- A new group takes no messages until its member adds finish: its composer says
  it is being created (RFGUI-102).
- App lock: a lock the daemon starts while the window is not in front now
  reaches the window (the closed subscription's `app_locked` refusal shows the
  lock page, and the lock state is read again when the daemon's countdown
  ends); the content stays covered until the latest foreground report is
  answered; focus changes are reported once the window settled; an unreadable
  lock state is read again with backoff while the account stays closed; a
  notification asked for during the lock opens its conversation after the
  unlock; the lock page names only a refusal of its own unlock.
- App lock: the read at the daemon's deadline is kept per account and issued by
  the session that heard the deadline (it never fired from the daemon session),
  only for an armed lock with time left; an `app_locked` refusal in an account
  session makes the daemon session list and read the locks again, so the lock
  page shows without any focus change (also for a lock sent by `rf`); the
  cover is given back when no report follows it or the daemon connection is
  lost, only covered accounts are told the window is back, and a burst of focus
  changes settles within one second.
- App lock: a lock sent by another client (`rf account app-lock lock`) now
  shows the lock page on an idle window at once. The window handles the
  daemon's `account.app_lock.changed` event on the account stream and on the
  daemon session's request connection (a locked event closes the account
  before any read; the lock is read again and the daemon session lists the
  accounts again). Another client's unlock re-arms a window in the background,
  an account is told the window is in front also when it was not covered
  (stored background or unknown), the daemon's accounts are listed again when
  an account session opens or finds its account not open, and a stream the lock
  refuses is retried with growing backoff up to 30 seconds.
- Agent approvals (G19): an approval is allowed only when the owner's roster
  holds its agent; an unknown agent can only be denied. An answered approval
  (`decided` with a decision) shows as allowed or denied.
- A PRX runtime action in Settings › Agents ends only with its own result:
  a failed background read no longer cancels an install or the daemon restart
  that adopts the runtime, a failed action reports its typed refusal, and a
  declined install confirmation clears the installing state.
- The agent roster keeps the journal cursor of the first page of a listing
  (later pages no longer move it), the pages after the first are applied, and
  a refused cursor (`journal_cursor_invalid`) is dropped and the roster listed
  again.
- Agents: a daemon restart after a runtime install or restart adopts the
  runtime from the answer to that action only, and every answer is a change,
  so an identical second restart still restarts the daemon once.
- Agents: the creation poll lives in the app, so it keeps following a
  creation while the panel is closed, and one `agent.operation.get` per
  request is in flight at a time. The roster follows the daemon's changes
  (`agent.watch`) after own changes, on reconnect and every 30 seconds, and
  reads every page of `agent.list`.
- Agents: retired agents are greyed out, and PRX refusals have their own
  wording (with a restart hint for an expired runtime credential or an
  unreachable runtime).
- New group stays in its creating state until every member has been added;
  the group conversation was shown (and a first message could be sent) while
  members were still being added, so the message reached only some of them.
- A failure banner ("Last error") no longer stays after the same operation
  succeeded (a node added after a refused attempt).
- A contact's "waiting for … to accept" row no longer stays after the
  request is no longer pending and the contact is accepted.
- The unread count no longer counts this account's own messages in the
  conversation shown (its read mark moves to the newest message).

- Attachments use the core's prepared-media path: the file is streamed into
  the daemon (`attachment.prepare.begin`, `.chunk`, `.finish`) and sent with
  `conversation.send`, in direct and group conversations, with the bytes
  moved shown as they go, Cancel and Retry. Received files are downloaded
  with Download (`attachment.import.*`) into the desktop's private media
  folder; images show inline, other files as file rows, Save copies them.
  A refused send says why; when the contact's home node is not pinned the
  row offers the contacts dock to exchange contact cards.
- Trust is a status (verified, not verified, identity changed); the safety
  number is an advanced detail. Verification reads the compared snapshot
  (`contact.safety_number`) and sends it to `contact.verify`.
- History: after the newest page only the rows changed after the read
  model's cursor are read; scrolling to the oldest shown message reads the
  older page. The conversation list shows the directory's preview while a
  conversation's history is not loaded.

- Core pinned to e39111de (the core's next head). Conversations are read
  from the message read model (`message.read`), group names are shared
  (`group.name.set`), the schema gate opens storage generation 1, and
  account creation and unlock ask for the six-digit account code. Agent
  surfaces show their fallback text until the card runtime is embedded.

### Removed

- The desktop's block renderer for agent surfaces (BlockKind blocks, patches,
  text streams, renderer conformance profiles and renderer error reports) and
  its payload translation. Surfaces are shown as their card (see Added) or
  their fallback text.
- The `RendererProfile` capability negotiation and the `surface.renderer_error`
  report: the card runtime draws every card, so there is nothing to report.
- The conformance fixtures and the scenarios that only exercised the removed
  renderer (`s11_surface_versions`, `s12_surface_limits`).
- The desktop's interim delivery-receipt sender (`message.receipt.delivered`):
  the core sends delivered receipts itself.
- The credential export of AI participants on the desktop (no request reaches
  the daemon for it any more).
- The retired one-shot attachment input (`message.submit` `attachments`), the
  object directory read and `object.get` / `object.transfer.resume`.

- Group invite and ban actions, joining by activation code, device sync
  import, the one-shot passphrase account form, the AI participant custody
  ceremony (agent creation is shown as unavailable), task event lines read
  from message text and the dormant `subscription.open` push path.

### Added

- Localisation: one JSON catalogue per locale (`locales/<tag>.json`) replaces
  the per-area two-locale YAML files; the build compiles them and fails on a
  key used in code but missing. Fifteen launch languages are selectable
  (English and Simplified Chinese complete, thirteen with English fallback
  and their own number and date formats), with BCP 47 matching of the system
  language, a locale → language → English fallback chain and CLDR plural
  rules for every category (Arabic and Russian included).
- Pseudo-locales `en-XA` (long, accented) and `ar-XB` (right to left) in
  developer builds.
- Right-to-left layout: the interface mirrors in Arabic through
  direction-aware rows, edges and chevrons; identifiers stay left to right;
  substituted untrusted text is bidi-isolated; message bubbles follow their
  content's direction. GPUI Kit components that do not mirror are listed in
  `docs/i18n.md`.
- Catalogue checks in `cargo test`: canonical catalogues with the source key
  set, unused keys, placeholder mismatches, a per-locale coverage report
  (`docs/i18n-coverage.md`) and a guard against physical edges in views.
- Design instrument (`crates/ramflux-desktop-design`): board reference
  manifest and verification, headless board captures over the mock source,
  pixel comparison with reviewed per-board deviations, token check by value
  against the design variables (`docs/design-instrument.md`).
- Design instrument wave W0: zh-CN board fixtures, attribution of
  deviations by measurement, kit component captures, the W1 work list.
- The call bar counts a call's duration on the session clock.
- Design scale (wave W1): spacing and type come from
  `ramflux_desktop_navigation::scale` (4 px grid, `rf-size-*` type sizes)
  instead of the Kit's rem-based utilities, which rendered 3.75/7.5/11.25 px
  spacing and 11.25/13.125/16.875 px type at the 15 px rem; seven `/ 16`
  rem divisors are fixed. `rf-design tokens --check` guards it in CI.
- Four screenshot programs skip with a message instead of failing when
  `RAMFLUX_SHOTS_DIR` is not set.
- Design instrument: a third accepted class, `stroke` (a centred design
  stroke against the kit's inside border, same edge colour and width, outer
  geometry within 1 px, measured), and kit regions attributed with the
  board's surroundings.
- Design checkbox and radio (board 00D): 18 px, the `rf-control` edge on
  the input fill, the `rf-primary` fill with its check, and the disabled
  `rf-surface` / `rf-line-strong` state; the checkbox keeps the UI kit's
  base checkbox for role, toggle state, focus and keyboard activation.
- Design text styles in `ramflux_desktop_navigation::scale`: each type size
  with its weight and line height (the interface face's natural line,
  measured on the boards), applied with `DesignText::design_text`; button
  labels use the button style.
- Text styles take the face of the interface language: Noto Sans SC first
  in Chinese, Japanese and Korean (the boards' `rf-font`, whose line metrics
  place CJK text 0.09 em lower than Hanken Grotesk's), Hanken Grotesk first
  otherwise.
- Design status chips and tip bars (boards 00B, 00D): chips in the neutral,
  success, warning and danger tones at 25 px; tip bars at 34 px with the
  tone's icon and one line of text. Settings notices are tip bars; the
  generated catalog `badge` is removed (one source).
- Design list row and rail item: md radius, the list row's title, time and
  preview in the design text styles, rail captions in the caption style
  ending in an ellipsis when long (the tooltip has the name). The rail's
  section items scroll in a short window, so Edit, Settings and the account
  stay reachable at the 640 x 480 minimum.
- Design message bubbles: a hand-written bubble shell (8 px above and
  below, 12 px across, the bubble radius) replaces the generated catalog
  bubble, whose empty quote, attachment and footer slots made every bubble
  12 px taller; message text on its 24 px line, the time in the caption
  style at the end of the last line, the sender in 13 px semibold, 32 px
  avatars, and the date separator at 20 px, 12 px above the day's first
  bubble.

- Linux identity: the window carries the application id
  `org.ramflux.desktop` (Wayland `app_id`, X11 `WM_CLASS`), the same id as
  the new desktop entry, hicolor icons (the macOS placeholder icon) and
  AppStream metainfo in `packaging/linux/`, checked by
  `scripts/validate-linux-metadata.sh` in CI and by a unit test against the
  macOS bundle id.
- One platform-directories module for every path: XDG config, data, state,
  cache and runtime directories on Linux (unset, empty or relative variables
  ignored), the macOS Library folders unchanged, and the conventional
  Windows locations named for later work.
- Linux release build: `scripts/package-linux.sh` produces
  `ramflux-desktop-<version>-linux-<arch>.tar.gz` and `SHA256SUMS`
  (release profile without demo scenarios, stripped with the build id kept,
  remapped build paths, file times from `SOURCE_DATE_EPOCH`); the
  `Linux release build` workflow builds x86_64 and aarch64 on Ubuntu 22.04
  from version tags or on demand, uploads artefacts and adds keyless build
  provenance on tags, and publishes nothing.
- `packaging/aur/ramflux-desktop-bin` (PKGBUILD and `.SRCINFO`) for x86_64
  and aarch64, not submitted.
- Operating-system notifications on Linux through the freedesktop interface
  with the `desktop-entry` hint, `transient` on any card that shows a name or
  text, urgency, one card per conversation and a click that opens the
  conversation; a per-device preview level (sender and text, sender only by
  default, nothing). Triggers and the Settings control come later.
- `docs/linux.md` and `docs/linux-verification.md`.

### Fixed

- Linux: the window has a title ("Ramflux"). It had none, so X11 window
  managers showed "Unnamed Window" and Wayland compositors, docks and task
  switchers an empty title.
- Linux: `scripts/package-linux.sh` finds the release binary in Cargo's
  target directory when `CARGO_TARGET_DIR` (or `build.target-dir`) is set,
  instead of failing on a missing `target/release/ramflux-desktop`.
- The first-run test for a daemon left behind by a crash no longer fails
  intermittently ("missing ElementId launch-socket"): it waits until the
  daemon's listings have routed the window before it opens Settings, so a
  late listing can no longer close Settings while the test scrolls it.
- The SDK calls tests wait on the bridge's completion notification instead of
  sampling every 15 ms, and no longer expect an exact number of `call.list`
  reads, which a recovery epoch on a loaded machine can exceed.
- The SDK calls tests no longer time out waiting for "head recovery calls":
  when the stream opens while the snapshot's `call.list` still waits for the
  schema gate, the head recovery joins that read, so only one `call.list` is
  sent. The tests waited for two. They now wait for a calls read after the
  stream subscription, answered, and a regression test forces the joined read
  through a new fake-bus answer hold.

### Changed

- Clean-up of public documentation: the manuals no longer carry internal task
  identifiers or round labels, `docs/demo.md` is a getting-started guide, and
  the example node address in the local network manual and the tests is a
  generic private one.
- The application icon is the Ramflux mark ("Flux Current") from the project
  website on every packaging path, replacing the placeholder glyph: Linux
  hicolor PNGs from 16 to 512 px plus scalable and symbolic SVGs, the macOS
  `.icns`, and a Windows `.ico`. All are generated from
  `packaging/brand/*.svg` by `scripts/generate-icons.sh`.
- On Linux a started daemon's default data folder is
  `$XDG_DATA_HOME/ramflux-desktop/rfd-data`. Data under the settings folder
  from older builds is not copied (no migrations before the beta).
- On Linux the default daemon socket is `$XDG_RUNTIME_DIR/ramflux/rfd.sock`
  (folder created 0700), unless a daemon already listens on the reference
  CLI's `/tmp/ramflux/rfd.sock`.
- On Linux the window draws no title bar of its own when the compositor
  decorates it or tiles it on every edge.
- Production code may not unwrap, expect, panic or take `std::sync` locks:
  workspace lints (`unsafe_code` forbidden, `dbg!` and `todo!` denied) and a
  deny in each crate root enforce it; a conventions test keeps every crate
  wired. The media rebind tag takes its HMAC key without a fallible call.
- The remaining `std::sync::Mutex` uses in production code (source wake,
  media engine and playback queue, app log, bus trace) are `parking_lot`
  locks.
- The demo scenarios, the mock source and the developer panel are behind the
  `demo` cargo feature (on by default for development and tests).
  `scripts/package-macos.sh` builds without it, so a packaged app carries no
  mock data and always runs on a daemon.
- CI checks the release configuration (no `demo` feature, no
  `ramflux-desktop-test` dependency), the licence header of every source
  file, and the DCO sign-off of the commits a pull request adds (earlier
  history is left as it is).
- README, data contract and demo checklist describe the current crates and
  layers; the demo checklist records where the daemon log and unlock findings
  stand.

## [0.2.6] - 2026-11-23

- Align release metadata with product version 0.2.6.

## [0.2.5] - 2026-11-22

- Version line aligned with product 0.2.5.

## [0.2.4] - 2026-11-21

- Version line aligned with product 0.2.4.

## [0.2.3] - 2026-11-21

- Version line aligned with product 0.2.3.

## [0.2.2] - 2026-11-21

- Version line aligned with product 0.2.2.

## [0.2.1] - 2026-11-17

- Version line aligned with product 0.2.1.

## [0.2.0] - 2026-11-16

First release under the shared Ramflux product version (all repos move together).
Changes since 0.1.0, at a high level:

- Settings, Diagnostics shows "Version 0.2.0 (build <commit>)" in the chosen language; every build now records its commit automatically.
- Refreshed account, device and backup settings.
- Tracks the generation-1 messaging of the product: the new message format, contact safety numbers and edit-conflict detection.

Details:

### Added

- Appearance: a theme picker (Light, Dark, Follow system) and a language
  picker (English, Simplified Chinese, Follow system) in Settings. Both apply
  at once in every window, are saved per device, and "follow system" tracks
  the operating system's appearance and preferred language. Themes are token
  sets: every `design/generated/<id>.json` the design compiler exports is
  registered at build time, with its RF roles from `rf-tokens.json`.
- Locale-aware formatting: plural messages (`one`/`other`), grouped numbers,
  percentages, sizes, date and time patterns, relative times, and label
  separators all come from the locale files. Checks fail on hard-coded
  user-visible strings and on any key-set difference between `en` and `zh-CN`.

- Event-driven desktop networking and send states: waiting messages retry only
  typed pre-acceptance refusals, show the scheduled countdown, and expire after
  ten minutes even when hidden. Manual retry is a new action; ambiguous outcomes
  retain the bus client's same-key recovery. English/Chinese light/dark state
  screenshots are generated and checked in macOS CI.

- Initial public AGPL Ramflux Desktop repository: Cargo workspace, GPUI Kit
  application entrypoint, and CI.
