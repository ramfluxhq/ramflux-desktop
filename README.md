# Ramflux Desktop

Ramflux Desktop is the native desktop client for Ramflux, built with
[GPUI Kit](https://gpui-kit.com). It connects to a local Ramflux daemon through
the SDK local bus and presents a conversation workspace for people, AI
participants, objects, and devices.

This repository contains only the AGPL client. The Ramflux core SDK and protocol
crates live in `ramfluxhq/ramflux` and are consumed as Git dependencies.

## Layout

```text
apps/ramflux-desktop          application entrypoint
crates/ramflux-desktop-core   models and local bus bridge (no GPUI types)
crates/ramflux-desktop-ui     GPUI Kit views, panels, theme
crates/ramflux-desktop-test   mock local bus and fixtures
```

## Requirements

Rust 1.99 (pinned in `rust-toolchain.toml`) and the GPUI platform
dependencies: macOS 15 or later with Xcode Command Line Tools; Windows 10 or
later with the MSVC toolchain; on Linux, the packages listed in the GPUI Kit
installation guide.

## Build and test

```sh
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo run
```

Live tests against a real `rf daemon` (no node needed) are ignored by
default. Install `rf` from the product revision pinned in `Cargo.lock`, then:

```sh
RAMFLUX_LIVE_DAEMON=1 RAMFLUX_RF_BIN=/path/to/bin/rf \
  cargo test -p ramflux-desktop-sdk -p ramflux-desktop-ui --test rfd_live -- --ignored --test-threads=1
```

Each test starts its own daemon in `/tmp/rf-f3-<pid>-<tag>` and stops it and
removes the directory afterwards.

## macOS app and demo

`scripts/package-macos.sh` builds `dist/Ramflux Desktop.app` (an ad hoc signed
development build). `docs/demo.md` walks through a demo on a real daemon and
node, and lists what the desktop supports today.

## Keyboard

Everything works from the keyboard; `secondary` is Command on macOS and
Control elsewhere.

| Keys | Command |
|---|---|
| Tab, Shift-Tab | Move between controls (overlays keep focus inside) |
| `secondary-1` … `secondary-6` | Show one of the first six visible conversations and focus the composer |
| `secondary-f` | Focus conversation search; Up/Down and Enter choose a result |
| `secondary-[` | Return to conversations in a compact window |
| `Shift-Enter` | Insert a line in the composer; Enter sends |
| `secondary-k` | Command palette (search contexts, people, objects, history) |
| `secondary-j` | Take me to what needs me (focuses the moment; Tab reaches its actions) |
| Enter | Send from the composer; activate the focused button |
| Enter or Space on Confirm | Confirm a high-risk request with one press (the strip focuses Confirm, Cancel comes first); Escape cancels |
| `secondary-shift-p` | Pin or unpin the shown context |
| `secondary-alt-1` … `secondary-alt-5` | Open or close the objects, contacts, members, approvals, evidence dock; Escape closes it |
| `secondary-,` | Settings; Ctrl-Tab and Ctrl-Shift-Tab change section; Escape closes |
| `secondary-shift-t` | Switch between light and dark |

## Licensing

Ramflux Desktop is licensed under AGPL-3.0-or-later with the Ramflux App Store
Exception. See `LICENSE`, `LICENSING.md`, and `NOTICE`.

Messages use source-provided quote, attachment, reaction and receipt facts. The
message menu copies current cached text locally. Reply, edit, reaction and delete
commands remain disabled until this connection supplies their mutation contracts.
Plain text supports native selection and copying; typed formatted spans retain
their existing renderer. Selection state is discarded with deletion, eviction,
account locking or a conversation switch.

Composer drafts are kept separately for each conversation and cleared with the
account session on lock/reset. Plain message selection uses native readonly text
controls so spaces, tabs and terminal blank lines copy exactly. Selection state
is allocated for visible rows; the cache protects the visible window plus overscan,
with a 64-entry warm-cache floor. Inactive drafts are capped at 128 and pruned
when their source conversation disappears.

Settings uses the generated account/device presentation and retains the native
masked secret inputs, backup jobs and host confirmation footer. At compact
widths its section navigation wraps above the scrolling page; section changes
cancel pending destructive confirmations. Device rows belong to the active
account. Missing authenticated platform/origin facts keep the operator-access
warning, including on the current or revoked device. Approving another device
and open-account privacy controls remain explicitly unavailable until their
source contracts exist; contact request acceptance is a separate setting.
