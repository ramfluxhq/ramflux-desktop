# Ramflux Desktop on Linux

Linux is a first-priority platform, with Omarchy (Arch Linux, Hyprland, uwsm)
as the main target. This page says what runs, how to install it, where files
live and what is not done yet. What still needs a check on a real machine is
listed in [linux-verification.md](linux-verification.md).

## Supported sessions

| Session | Status |
|---|---|
| Omarchy (Hyprland, Wayland), x86_64 | target; to be verified on a real machine |
| Omarchy edge, aarch64 | built in CI; to be verified |
| Arch Linux with GNOME or KDE Plasma (Wayland) | expected to work; to be verified |
| Other distributions with glibc 2.35 or newer (Ubuntu 22.04+, Fedora) | the release archive is built on Ubuntu 22.04 |
| X11 sessions | supported by GPUI (XIM input); best effort |

GPUI renders through Vulkan, with a GLES path in the pinned version; a
machine or virtual machine without either cannot run the app.

## Install

### From the release archive

`scripts/package-linux.sh` builds
`ramflux-desktop-<version>-linux-<arch>.tar.gz` with a `SHA256SUMS` file; the
release workflow (`.github/workflows/release-linux.yml`) builds both
architectures from a version tag. The archive's `README.md` gives the copy
commands for a system-wide or per-user install.

### On Arch Linux and Omarchy

`packaging/aur/ramflux-desktop-bin/PKGBUILD` repackages the release archive
under `/usr`. Until a release with these archives exists, fill in `pkgver`
and the two checksums from `SHA256SUMS`, then:

```sh
cd packaging/aur/ramflux-desktop-bin
makepkg --printsrcinfo > .SRCINFO   # the committed .SRCINFO is hand-written
makepkg -si
namcap PKGBUILD ramflux-desktop-bin-*.pkg.tar.zst
```

Nothing is submitted to the AUR from this repository.

### Package dependencies

`depends` in the PKGBUILD is derived from what the GPUI Linux platform, the
audio stack and the font stack need; it has not been checked against a built
binary yet. Before a submission, on the released archive:

```sh
ldd usr/bin/ramflux-desktop          # libraries linked at start
namcap ramflux-desktop-bin-*.pkg.tar.zst
```

The release workflow saves the `ldd` output of each architecture
(`ldd-linux-<arch>.txt`). Libraries GPUI opens at run time (Vulkan, and
possibly Wayland or X11 pieces) do not appear in `ldd`; keep
`vulkan-icd-loader`, `wayland` and `libxkbcommon-x11` regardless.

### Running the daemon

The desktop needs a Ramflux daemon (`rf daemon start`). A `rf` next to
`ramflux-desktop` (for example both in `/usr/bin` from two packages) is found
by "Start local daemon" without a setting; otherwise set the program in
Settings › Diagnostics.

## Application identity

One id, `org.ramflux.desktop`, is used for the Wayland `app_id` and X11
`WM_CLASS` of the window, the desktop entry
(`/usr/share/applications/org.ramflux.desktop.desktop`, `StartupWMClass`),
the icon name, the AppStream id, the notification `desktop-entry` hint and
the macOS bundle id. The constant is
`ramflux_desktop_sdk::local::identity::APP_ID`; a unit test checks the
packaging files against it. Hyprland window rules match it:

```lua
o.window("org.ramflux.desktop", { ... })
```

The icons in `packaging/linux/icons/` are the same placeholder the macOS
package builds (the Lucide "orbit" glyph on a dark tile, ISC licence) until
the final icon arrives from design; there is no symbolic variant yet.

`scripts/validate-linux-metadata.sh` checks the desktop entry with
`desktop-file-validate`, the metainfo with `appstreamcli validate --strict`,
and the icon set; it skips a validator that is not installed (CI installs
both and requires them).

## Where files live

All paths come from one module,
`crates/ramflux-desktop-sdk/src/local/platform_dirs.rs`, laid out by
`local/paths.rs`. XDG variables that are unset, empty or relative are
ignored, as the Base Directory specification requires.

| What | Path |
|---|---|
| settings, dock layout, record of a started daemon | `$XDG_CONFIG_HOME/ramflux-desktop/` (`~/.config/ramflux-desktop/`) |
| daemon data (default) | `$XDG_DATA_HOME/ramflux-desktop/rfd-data/` (`~/.local/share/...`) |
| logs (app and started daemon) | `$XDG_STATE_HOME/ramflux-desktop/logs/` (`~/.local/state/...`) |
| daemon socket (default) | `$XDG_RUNTIME_DIR/ramflux/rfd.sock` |

`RAMFLUX_DESKTOP_STATE_DIR` and `RAMFLUX_DESKTOP_LOG_DIR` still override the
settings and log folders; with the first set, daemon data stays under it and
the socket stays the reference CLI's, as before.

### Daemon data moved out of the settings folder

Older builds put a started daemon's data under
`~/.config/ramflux-desktop/rfd-data`. On start, before the window opens, the
desktop copies it to `~/.local/share/ramflux-desktop/rfd-data`
(`local/data_migration.rs`), unless a data folder is set in Settings ›
Diagnostics:

1. If the new folder exists, nothing happens (the copy is done).
2. The copy holds the daemon's own lock (`daemon.lock`) on the old folder for
   its whole duration. If a daemon is using the old folder, nothing is copied
   and the old folder stays in use; the next start tries again.
3. The tree is copied to `rfd-data.migrating` beside the destination, synced,
   and compared byte for byte with the original. A symbolic link or special
   file, a difference or an I/O error abandons the copy and removes only that
   staging folder.
4. The staging folder is renamed to `rfd-data`; that rename is the switch.
   `rfd-data.migrated-from` records where the data came from.
5. The old folder is never changed or deleted. Remove
   `~/.config/ramflux-desktop/rfd-data` yourself once the app works from the
   new place.

Until the switch, the old folder is the default, so a skipped or failed copy
never hides data. The copy runs synchronously at start; a very large data
folder delays the first window of that one start.

### The daemon socket

Without a socket set, the desktop connects to
`$XDG_RUNTIME_DIR/ramflux/rfd.sock` and starts its daemon there
(`rf --socket <path> daemon start ...`), first creating
`$XDG_RUNTIME_DIR/ramflux` with mode 0700. Two exceptions keep existing
setups working:

- If nothing is at the runtime socket but a daemon listens at the reference
  CLI's default, `/tmp/ramflux/rfd.sock` (`rf daemon start` without
  `--socket`, or one an older desktop started), the desktop attaches there.
- A runtime path longer than a Unix socket address allows falls back to the
  CLI's socket.

Without `$XDG_RUNTIME_DIR` the socket goes to
`~/.local/state/ramflux-desktop/run/rfd.sock` (a folder of mode 0700).

`rf` and `rf-tui` still default to `/tmp/ramflux/rfd.sock`: to reach a
daemon the desktop started, give them `--socket
"$XDG_RUNTIME_DIR/ramflux/rfd.sock"`, or start the daemon yourself with
`rf daemon start` and let the desktop attach to it. Moving the CLI's default
is a question for the daemon owners (see below).

## Notifications

Operating-system notifications go through the freedesktop notifications
interface (`crates/ramflux-desktop-ui/src/notify/`), which Omarchy's shell,
mako, dunst, GNOME and KDE implement:

- `desktop-entry` hint `org.ramflux.desktop`, so the server shows the app's
  icon and groups its cards;
- **`transient` on every notification that shows a name or text**: servers
  that keep a history on disk (Omarchy keeps a day of cards in
  `history.json`) never store such a card;
- urgency `critical` for incoming calls, `normal` otherwise;
- one card per conversation and kind: a newer message replaces the card of
  the older one;
- a `default` action: clicking the card brings the window forward and opens
  the conversation.

**Preview privacy** is a per-device setting kept in the settings file
(`notifications.preview`):

| Level | Shows | Transient |
|---|---|---|
| `sender_and_text` | the sender and up to 160 characters of text | yes |
| `sender_only` (default) | the sender and "New message" | yes |
| `nothing` | "New message" | no |

The default is `sender_only`: knowing who wrote is enough to decide whether
to look, and no encrypted text ever reaches the notification server. Text
previews are cleaned of control characters, and escaped when the server
interprets markup.

Not done yet: what raises a notification (a message while the window is not
focused, mute rules, approvals, calls) and the Settings control for the
level both come with the notifications work package; the sending side and
the click-through are in place. macOS and Windows show nothing for now.

## Window chrome on tiling compositors

The window asks for server-side decorations (GPUI's default). Where the
compositor decorates the window (Hyprland, Sway, KDE; Omarchy's floating
workspaces draw a themed title bar), or tiles a client-decorated window on
all four edges, the app draws no title bar of its own
(`crates/ramflux-desktop-ui/src/chrome.rs`). A floating, maximized or
half-snapped window without server decorations (GNOME) keeps the bar: it is
the only way to move it and holds the close button. The app sets no minimum
assumption about its size beyond 640×480.

## Theme following

Light and dark follow the desktop through the Settings portal
(`org.freedesktop.appearance color-scheme`), which Omarchy's theme switch
sets. Following Omarchy's accent and surface colours is a later work package.

## Secret storage

The desktop holds no keys; the daemon does. Findings at the pinned core
revision (`b847923b`) and at current core:

- `rf vault init --mode <systemd-host|systemd-tpm2|desktop-secret-service>
  --vault-root <dir>` exists and initialises a Linux vault installation
  (`apps/rf/src/handlers/vault.rs`); "availability never changes the selected
  mode" (`custody/vault/linux/config.rs`).
- The daemon does not use it: `rf daemon start` has no vault option, and its
  primary-key provider is `NativePrimaryVault`, which on Linux refuses every
  operation with `CustodyVaultUnavailable` (`custody/vault.rs`,
  `bus/custody_setup.rs`). The Linux providers (`LinuxPrimaryVault`) are used
  only by `rf`'s own tests.

So there is no supported call the desktop could make today that changes
where keys are kept, and nothing is implemented. Proposal once the daemon
selects a Linux vault:

1. **When**: on the first "Start local daemon" with no account yet, before
   the account ceremony.
2. **Call**: `rf vault status --mode desktop-secret-service --vault-root
   <data>/vault` and, if it is not initialised, `rf vault init` with the same
   arguments, then `rf daemon start` with whatever option selects that vault.
3. **What the user sees**: a step "Keep your keys in the system keyring"
   naming the keyring (gnome-keyring or KWallet), the keyring's own unlock
   prompt, then the ceremony.
4. **Without a keyring** (no `org.freedesktop.secrets` on the session bus,
   or a locked keyring the user cancels): no silent fallback to a weaker
   mode; the step explains how to install or unlock a keyring and offers to
   retry. `systemd-host` is a headless choice and is not offered by the
   desktop.

## Signatures

Release archives get a `SHA256SUMS` file and, on tag builds, a keyless
GitHub build-provenance attestation (`gh attestation verify <archive> --repo
ramfluxhq/ramflux-desktop`). A detached PGP or minisign signature over
`SHA256SUMS`, which distribution recipes verify with offline keys, waits for
the owner's release-key decision; the workflow marks the place.

## Known limits

- Not built or run on Linux by the author of this support: everything in
  [linux-verification.md](linux-verification.md) is open.
- Placeholder icon; no symbolic icon.
- Notification triggers and the privacy control in Settings are not wired.
- Clicking a notification on Wayland relies on the compositor accepting the
  window activation; GPUI requests it without the server's activation token.
- No tray icon (by design), no autostart entry, no systemd user unit yet.
- File dialogs start in `~/Documents`; localised XDG user folders
  (`user-dirs.dirs`) are not read.
- Daemons started by the desktop and the CLI use different default sockets
  (above) until the CLI moves.

## Questions for the daemon owners

1. Default socket: will `rf` and `rf-tui` default to
   `$XDG_RUNTIME_DIR/ramflux/rfd.sock` (falling back to a private folder when
   it is unset) instead of `/tmp/ramflux/rfd.sock`? The desktop already uses
   that path for daemons it starts.
2. Default data root: will `rf daemon start` default to
   `$XDG_DATA_HOME/ramflux/...` instead of `/tmp/ramflux/rfd-data`? Data in
   `/tmp` is lost at reboot on most distributions.
3. Socket folder: the daemon creates the socket's parent with the process
   umask (often 0755) and deletes whatever file is at the socket path before
   it binds. Should it create the folder 0700, refuse a folder owned by
   someone else, and refuse to unlink a socket another live daemon answers
   on?
4. Vault: when will `rf daemon start` select a Linux vault (an option or a
   recorded mode), so the desktop can offer `desktop-secret-service`? Which
   `--vault-root` layout should the desktop use, and must it be outside the
   data root?
5. Linux aarch64 and a packaged `rf` (`ramflux-cli-bin`) for the
   `ramflux-desktop-bin` dependency: who builds and maintains them?
