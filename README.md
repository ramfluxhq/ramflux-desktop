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

## Licensing

Ramflux Desktop is licensed under AGPL-3.0-or-later with the Ramflux App Store
Exception. See `LICENSE`, `LICENSING.md`, and `NOTICE`.
