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

## Keyboard

Everything works from the keyboard; `secondary` is Command on macOS and
Control elsewhere.

| Keys | Command |
|---|---|
| Tab, Shift-Tab | Move between controls (overlays keep focus inside) |
| `secondary-1` … `secondary-6` | Show a context of the stack and focus the composer |
| `secondary-k` | Command palette (search contexts, people, objects, history) |
| `secondary-j` | Take me to what needs me (focuses the moment; Tab reaches its actions) |
| Enter | Send from the composer; activate the focused button |
| Space or Enter, held | Hold to confirm a high-risk request; Escape cancels |
| `secondary-shift-p` | Pin or unpin the shown context |
| `secondary-alt-1` … `secondary-alt-5` | Open or close the objects, contacts, members, approvals, evidence dock; Escape closes it |
| `secondary-,` | Settings; Ctrl-Tab and Ctrl-Shift-Tab change section; Escape closes |
| `secondary-shift-t` | Switch between light and dark |

## Licensing

Ramflux Desktop is licensed under AGPL-3.0-or-later with the Ramflux App Store
Exception. See `LICENSE`, `LICENSING.md`, and `NOTICE`.
