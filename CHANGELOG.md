# Changelog

All notable changes to this project will be documented in this file.

The format is based on Keep a Changelog, and this project adheres to Semantic
Versioning.

## [Unreleased]

### Added

- Message actions in direct and group conversations: Reply (the composer
  quotes the message; the bubble shows the quote), Edit (with the revision
  the core expects; the message is marked edited), Delete for everyone
  (confirmed on the strip) and Copy, from the message's actions button, its
  context menu or the keyboard. They use the core's conversation mutations
  (`conversation.reply`, `.edit`, `.delete`).
- Read receipts: the conversation shown is marked read through its newest
  message (`message.receipt.read`; the core routes the receipt to the
  author's device), and own messages show Read when the peer read them.
- Conversation names from the core (`peer_display_name` on direct rows).
- Leave group (the conversation's menu, confirmed) and group invitations
  from people who are not contacts: listed in the contacts dock with Join
  and Decline (`group.invitations.list`, `group.invitation.accept` and
  `.decline`, the `group.invitation` event).
- Usernames: Settings › Account shows the username the directory reports
  or why it is unavailable; Add contact accepts `@name`
  (`username.contact.add`).

### Changed

- Core pinned to 5d117366 (the core's next head). The username directory is
  provisioned by `rf daemon start` from its environment (`RAMFLUX_USERNAME_CENTER_*`,
  `RAMFLUX_USERNAME_TRUST_GENERATION`); the desktop passes its environment on.
- A direct conversation's header no longer shows a member count.
- Adding a contact card confirms ("Contact card added.") also when the card
  named a contact that was already known.
- The AI marker follows the core's signed classification
  (`verified_classification`: person, AI agent, unknown); an unproven claim
  no longer draws a party as an agent.

### Fixed

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
