# Changelog

All notable changes to this project will be documented in this file.

The format is based on Keep a Changelog, and this project adheres to Semantic
Versioning.

## [Unreleased]

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
- On Linux a started daemon's default data folder is
  `$XDG_DATA_HOME/ramflux-desktop/rfd-data`; data under the settings folder
  from older builds is copied there once (under the daemon's lock, verified
  byte for byte, then switched; the old folder is never deleted).
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
