# Changelog

All notable changes to this project will be documented in this file.

The format is based on Keep a Changelog, and this project adheres to Semantic
Versioning.

## [Unreleased]

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
