# Linux verification checklist

The Linux support in [linux.md](linux.md) was written and unit-tested on
macOS: the Linux-only code (`crates/ramflux-desktop-ui/src/notify/linux.rs`,
the `cfg(target_os = "linux")` dependency on `notify-rust`) has **not been
compiled**, and nothing has run on Linux. Run this list on:

- **A**: Omarchy stable (Hyprland, uwsm), x86_64, real hardware or a VM;
- **B**: Omarchy edge, aarch64 (for example Try Omarchy on Apple silicon);
- **C**: a generic Wayland session (Arch or Fedora with GNOME);
- **D**: an X11 session (for example i3 or Xfce).

Record for each item: session, result (pass, fail, n/a), evidence (command
output or screenshot) and the build (`ramflux-desktop --version` is not
available; use Settings › Diagnostics or the archive name).

## 0. Build and package

| # | Check | How | A | B | C | D |
|---|---|---|---|---|---|---|
| 0.1 | The workspace compiles and tests pass on Linux | `cargo clippy --workspace --all-targets --all-features -- -D warnings`; `cargo test --workspace` (CI does this on ubuntu-24.04) | | | | |
| 0.2 | The archive builds | `scripts/package-linux.sh`; note the printed glibc version | | | | |
| 0.3 | The archive is reproducible | build the same commit twice (clean `target/`), compare `sha256sum` | | | | |
| 0.4 | No build path in the binary | `strings usr/bin/ramflux-desktop \| grep -E '/home/\|/Users/'` prints nothing | | | | |
| 0.5 | The build id survives stripping | `readelf -n usr/bin/ramflux-desktop \| grep 'Build ID'` | | | | |
| 0.6 | Metadata validates | `REQUIRE_VALIDATORS=1 scripts/validate-linux-metadata.sh` | | | | |
| 0.7 | The package builds and is clean | fill in `pkgver` and checksums; `makepkg -si`; `namcap PKGBUILD *.pkg.tar.zst` | | | | |
| 0.8 | `depends` is complete and minimal | compare `ldd usr/bin/ramflux-desktop` and `namcap` output with the PKGBUILD | | | | |
| 0.9 | Removal is clean | `pacman -Rns ramflux-desktop-bin`; no file left under `/usr` | | | | |

## 1. Identity and launcher

| # | Check | How | A | B | C | D |
|---|---|---|---|---|---|---|
| 1.1 | The compositor sees the app id | Hyprland: `hyprctl clients` shows `class: org.ramflux.desktop`; X11: `xprop WM_CLASS` shows `org.ramflux.desktop` twice | | | | |
| 1.2 | Launcher entry and icon | Omarchy launcher (Super + Space) lists "Ramflux" with the icon; GNOME overview search finds it, also by "加密" in a zh_CN session | | | | |
| 1.3 | The running window is matched to its entry | the dock or task switcher shows the Ramflux icon, not a generic one | | | | |
| 1.4 | Software centre | GNOME Software or KDE Discover shows the metainfo (summary, licence, links) for the installed package | | | | |

## 2. Files

| # | Check | How | A | B | C | D |
|---|---|---|---|---|---|---|
| 2.1 | Fresh install layout | after first run: settings in `~/.config/ramflux-desktop/`, logs in `~/.local/state/ramflux-desktop/logs/`, nothing in `/tmp` except a CLI daemon's own files | | | | |
| 2.2 | Socket in the runtime directory | "Start local daemon"; `ls -ld $XDG_RUNTIME_DIR/ramflux` shows `drwx------`; `rfd.sock` inside | | | | |
| 2.3 | Attach to a CLI daemon | start `rf daemon start` (socket `/tmp/ramflux/rfd.sock`) first, then the app: it attaches | | | | |
| 2.4 | No runtime directory | `env -u XDG_RUNTIME_DIR ramflux-desktop`: the socket goes to `~/.local/state/ramflux-desktop/run/` (mode 0700) | | | | |

## 3. Appearance and input

| # | Check | How | A | B | C | D |
|---|---|---|---|---|---|---|
| 3.1 | Light/dark follows the system | Omarchy: Super + Ctrl + T to a light and a dark theme; GNOME: Appearance; the app switches without restart | | | | |
| 3.2 | Fractional scaling | Hyprland `monitor` scale 1.25, 1.5, 1.75, 2.0 (GNOME: experimental fractional scaling): text is sharp, layout not clipped | | | | |
| 3.3 | Input method | fcitx5 pinyin (and mozc if installed): compose Chinese and Japanese in the composer; the candidate window sits at the caret | | | | |
| 3.4 | Fonts | CJK, emoji and Arabic render (system fallback) | | | | |

## 4. Notifications

Notification triggers are not wired yet; until they are, exercise the sender
from a debug build (for example a temporary call to
`ramflux_desktop_ui::notify::post` from the developer panel) and check:

| # | Check | How | A | B | C | D |
|---|---|---|---|---|---|---|
| 4.1 | The card shows the app icon and name | the card uses the Ramflux icon (from the `desktop-entry` hint) | | | | |
| 4.2 | Privacy levels | at each level the card shows only what [linux.md](linux.md) lists | | | | |
| 4.3 | Message content is never persisted | Omarchy: after a `sender_only` and a `sender_and_text` card, `grep` the sender and text in `~/.local/state/omarchy/notifications/history.json`: absent; a `nothing` card may be present | | | | |
| 4.4 | Click-through | clicking the card focuses the window (also from another workspace) and opens the conversation | | | | |
| 4.5 | Replacement | two messages in one conversation leave one card; two conversations leave two | | | | |
| 4.6 | Calls are urgent | a call card stays until handled (critical urgency) | | | | |
| 4.7 | No notification server | stop the server (or run without a session bus): the app keeps working; one warning per attempt in the log | | | | |

## 5. Desktop integration

| # | Check | How | A | B | C | D |
|---|---|---|---|---|---|---|
| 5.1 | File dialogs through the portal | Settings › Diagnostics export: the portal dialog opens (Omarchy: Folio or GTK) | | | | |
| 5.2 | Tiling | Hyprland tiled: no in-app title bar row, no duplicate buttons; resizing by the tiling layout works at any size down to 640×480 | | | | |
| 5.3 | Floating | Omarchy 4.5 floating workspace: the compositor's themed title bar only; GNOME floating window: the app's bar with working close, drag and double-click | | | | |
| 5.4 | Half-snapped and maximized | GNOME snapped left and maximized: the app's bar stays | | | | |
| 5.5 | Decoration changes at run time | toggle floating/tiled (Hyprland Super + T or similar): the bar appears and disappears without a restart | | | | |
| 5.6 | Keyring | `secret-tool search --all` lists no Ramflux item today (the daemon does not use the Secret Service yet; see linux.md) | | | | |
| 5.7 | Window rules | an Omarchy rule for `org.ramflux.desktop` (for example `focus_on_activate = false`) applies | | | | |

## 6. Calls

| # | Check | How | A | B | C | D |
|---|---|---|---|---|---|---|
| 6.1 | Devices | microphone and speaker listed (PipeWire through ALSA); a call desktop-to-desktop has audio both ways | | | | |
| 6.2 | Device change | plug a headset during a call: audio moves or the change is shown | | | | |
| 6.3 | Screen stays on | a 30-minute call does not blank the screen (idle inhibit is not implemented yet; expected to fail) | | | | |

## 7. Release workflow

| # | Check | How |
|---|---|---|
| 7.1 | Manual run | run "Linux release build" from the Actions tab on a branch: both architectures build; artefacts `linux-release` holds two archives, `SHA256SUMS`, two `ldd` files |
| 7.2 | arm64 runner | the `ubuntu-22.04-arm` job finds or installs rustup and the pinned toolchain |
| 7.3 | Tag run | on a test tag in a fork: provenance attestation created; `gh attestation verify` passes; no release or tag is created by the workflow |
| 7.4 | Checksums | `sha256sum -c SHA256SUMS` passes on the downloaded files |
