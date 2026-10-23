# Demo checklist: Ramflux Desktop on macOS

A walk through the desktop client on a real local daemon and a real node:
build the app, start the daemon, add a node, create an account, add a contact
and send a message. Each step says what the desktop does today and what still
waits for daemon support. Nothing in this list is simulated; controls the
daemon cannot serve yet are shown disabled with the reason.

## 1. Build the app

Requirements: macOS 15 or later, Xcode Command Line Tools, the Rust toolchain
pinned in `rust-toolchain.toml`.

```sh
scripts/package-macos.sh                 # dist/Ramflux Desktop.app
scripts/package-macos.sh --rf /path/rf   # also ship an rf binary in the app
open "dist/Ramflux Desktop.app"
```

The script builds a release binary (`cargo build --release --locked`), writes
`Info.plist` (bundle id `org.ramflux.desktop`, version from `Cargo.toml`), a
placeholder icon and the license notices, and signs the bundle ad hoc. An ad hoc
signature only lets the bundle run on the machine that built it.

Not done yet (later release work): Developer ID signing with the hardened
runtime, notarization and stapling, a DMG, and auto-update.

`rf` is the reference CLI and daemon from `ramfluxhq/ramflux`. Install it from
the revision pinned in `Cargo.lock`, for example:

```sh
cargo install --git https://github.com/ramfluxhq/ramflux --rev <pinned rev> --locked ramflux-rf
```

## 2. Start the daemon

A release build uses the local daemon by default and attaches to the socket of
the reference CLI, `/tmp/ramflux/rfd.sock`. The app never starts a daemon by
itself. Pick one:

- **Already running:** start it yourself, `rf daemon start` (add
  `--socket PATH --data-root DIR` to choose both). The app attaches as soon as
  the socket answers.
- **From the app:** on the first screen press **Start local daemon**. The app
  runs `rf --socket <socket> daemon start --data-root <data folder>` and
  attaches. It uses the `rf` shipped in the app, or the program set in
  **Settings › Diagnostics**. Without either, the button is disabled and the
  page explains where to set it. A start that fails (program missing, not
  executable, exits at once) is shown with the reason and can be retried.

A daemon started by the app belongs to the app: **Stop daemon** in Settings ›
Diagnostics stops it, and quitting the app stops it. The app asks the daemon
to stop (it finishes what it is writing, closes its data and removes its
socket) and only terminates the process if it refuses or has not exited after
a few seconds. If the app ended without stopping it (a crash), the next start
attaches to it and Settings › Diagnostics offers **Stop daemon** for it. A
daemon you started yourself is never stopped by the app; `rf daemon stop`
stops it.

## 3. Settings › Diagnostics

| Setting | Default | Applies |
|---|---|---|
| Use the local daemon (off: demo scenarios) | on in release builds | next app start |
| Daemon socket | `/tmp/ramflux/rfd.sock` | next app start |
| Daemon program (`rf`) | the one shipped in the app, else none | next daemon start |
| Daemon data folder | `~/Library/Application Support/Ramflux Desktop/rfd-data` | next daemon start |

The same section shows the daemon state, the version (app, product revision,
GPUI Kit), the log and settings file locations, the redacted diagnostics export
and the renderer profile.

Command line and environment override the saved choices for one run:
`--source mock|rfd`, `--socket PATH`, `RAMFLUX_DESKTOP_SOURCE`,
`RAMFLUX_DESKTOP_SOCKET`.

## 4. Add a node by URL

On the node step type the node's discovery URL and press **Look up**. The
node's signed record is checked before anything is saved; the card shows its
endpoints, fingerprint and expiry. A node that is not from an invitation is
trusted on first use, which needs a hold-to-confirm. Supported today.

## 5. Create an account

On the account step choose **Create account**, name it, set a passphrase and
create. The workspace opens once the daemon reports the account unlocked.
Unlock, lock, passphrase change and restore from a backup file are supported.
Joining as a new device with an activation code is not available from the
daemon yet (shown disabled).

## 6. Add a contact

Open the contacts dock (`secondary-alt-2`, or **Add a contact…** in the command
palette), paste the id the contact shared with you (their principal id) and
press **Add**. The contact is added on this account at once; the field explains
an id it cannot use, and a daemon error is shown under the field. Supported
today.

Adding does not send the contact a request: a request travels to the contact's
node over a federation route, which the daemon does not list for a contact.
Send one with the reference CLI against the same socket (`rf contact request …`;
see `rf contact --help`).

A request appears under **Requests** in the dock within one poll once the
daemon has recorded it, which it does when it receives the request message in
that conversation (the desktop receives only the conversations you have open or
pinned; `rf dm read …` receives others). **Accept** adds the contact on
this account and **Reject** declines.
The requester is not told yet, for the same reason. A contact's menu blocks or
removes them (both revoke what you shared and need a hold-to-confirm); blocked
contacts can be unblocked. **Verify** compares the contact's devices through the
node and marks the safety number verified (it needs the node, and the contact's
identity, which the daemon lists once it has fetched their device manifest, for
example after a message).

## 7. Send a message or an attachment

| What | Today |
|---|---|
| Message in a group | Supported. The daemon resolves every member's route first; if a member cannot be reached nothing is sent, and the failure names who. Incoming messages are fetched by polling (1.5 s, slowing to 12 s when quiet). |
| Direct message | Supported in a conversation the daemon lists a route for (recorded once a message was exchanged with the contact's verified device). Without one the send fails with that reason; start the conversation with `rf dm send`. |
| Attachment | Supported in direct conversations, up to 512 KiB per file (one message frame). Sent attachments are listed in the objects dock, where they can be saved and a paused upload resumed. Received attachments are not downloaded yet: receiving never waits on an attachment, and the daemon has no way to fetch one after its message arrived (pending daemon support). Groups carry no attachments. |
| Group members | Add or invite a contact whose devices the daemon has verified (members dock); remove, kick, ban and roles act on the member's devices in the group. |
| Approvals | An approval requested from a conversation appears in that conversation and in "needs me"; the others in the approvals dock only. |

## Where the app keeps files

| What | Location |
|---|---|
| Settings (launch choices, pinned contexts) | `~/Library/Application Support/Ramflux Desktop/desktop-settings.json` |
| Dock layout | `~/Library/Application Support/Ramflux Desktop/dock-layout.json` |
| App log (redacted, rotated at 1 MiB, 3 kept) | `~/Library/Logs/Ramflux Desktop/desktop.log` |
| Output of a daemon the app started | `~/Library/Logs/Ramflux Desktop/rfd.log` |
| Data of a daemon the app started | the daemon data folder above |

The app log holds only the app's own events (start, source, daemon start and
exit, failures). Values of secret-looking fields, long opaque tokens and PEM
blocks are redacted before they are written. Account keys and messages stay in
the daemon's data folder; deleting the app bundle does not remove them.

## Reset

Quit the app (this stops a daemon it started), then remove the files above.
To start from scratch with a daemon you run yourself, stop it and remove its
data root.
