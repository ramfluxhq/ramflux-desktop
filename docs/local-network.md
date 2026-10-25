# macOS Local Network privacy and the local daemon

Since macOS 15, a program may reach hosts on the local network (for example a
node at `http://192.168.31.136:59482`) only after the user allows it in
System Settings › Privacy & Security › Local Network. This note records how
the desktop and the daemon it starts fit those rules, what went wrong in
DESK-F19, and how to test it. The reference is Apple's
[TN3179: Understanding local network privacy](https://developer.apple.com/documentation/technotes/tn3179-understanding-local-network-privacy).

## Who asks, and for whom

"Start local daemon" runs `rf --socket <socket> daemon start --data-root
<dir>` as a plain child process of the app (`std::process::Command`, which is
`posix_spawn`): no new session, no double fork, no launchd job. For such a
helper macOS looks up the *responsible code*, the app that spawned it, and
uses the app for the prompt, for the recorded choice and for the entry in
System Settings (TN3179, "macOS considerations"). So:

- The daemon needs no permission, usage string or entitlement of its own; the
  app's `NSLocalNetworkUsageDescription` (in `Info.plist`, written by
  `scripts/package-macos.sh`) is the one shown.
- `NSBonjourServices` is not needed: neither the app nor the daemon browses
  or advertises Bonjour services; they connect to addresses the user typed.
- The daemon must stay the app's child. A daemon that detached itself, or ran
  as a launchd agent, would no longer be covered by the app (a launchd agent
  would need `AssociatedBundleIdentifiers`). `tests/daemon_process.rs` in
  `ramflux-desktop-sdk` checks that the started daemon is the app's child in
  the app's process group.
- A daemon started from Terminal or over SSH is covered by those instead
  (exempt), which is why `rf` from a terminal reaches the node while the app's
  daemon may not.

The unified log confirms the attribution: for a connection made by the
bundled `rf`, `UserEventAgent` logs `LocalNetwork: found bundle id
org.ramflux.desktop by PID`. A test bundle on the owner's Mac, with the same
bundle id and a second executable spawned the same way, connected to the LAN
node from the helper as well as from the main executable once the app was
allowed.

## What went wrong (DESK-F19)

macOS keeps the allowed app as a rule with the Mach-O UUIDs of its main
executable (TN3179, "Build-time considerations"), and policies are installed
for those UUIDs. Every build of `ramflux-desktop` has a new UUID. After an
update the rule still lists the previous build's UUID, so:

1. The first local network connection after the update is dropped at once by
   the kernel (`tcp drop outgoing ... reason: NECP`, `EHOSTUNREACH`), even
   though the switch in System Settings is on.
2. That drop is what makes `nehelper` notice (`Local network allowed by
   preference for Ramflux Desktop (org.ramflux.desktop), but received prompt.
   Clearing cached UUIDs and restarting session.`) and record the new UUID.
3. From then on connections go through.

On 2026-10-25 the log showed exactly this: the rule held UUID `C4605AE6…`
(an earlier build), the daemon's connections at 10:44:35 and 10:45:24 were
dropped, the second one refreshed the rule to the installed build's
`3B501E6B…`, and no lookup was tried after that. The daemon reports the drop
only as `error sending request for url (...)`, so the user saw a failure
right after allowing the app, and toggling the switch could not help: the
switch was already on; the stale UUID was the problem. The same immediate
failure happens while the first-use prompt is still open.

An ad hoc signature makes this worse but is not the cause: TN3179 notes that
the system tracks a program by its code signature and that only an
Apple-issued identity keeps it reliable. With ad hoc signing each build is a
new code identity.

## The fix

- **Repeat a blocked lookup.** A node lookup (`node.add`, `node.probe`) that
  fails the way a Local Network block does on macOS (an unreachable host, or a
  failed request to a private, link-local or `.local` host) is repeated after
  2 s and again after 4 s, each time as a new attempt with a fresh retry key
  (`bridge/effects.rs`, `SdkConfig::local_network_retry_ms`). The blocked
  attempt is what makes macOS refresh its rule or show its prompt, so the
  repeat goes through without restarting the app or the daemon. If it still
  fails, the node step explains the permission and where to grant it, and
  **Look up** tries again.
- **Sign with a stable identity when one is available.**
  `scripts/package-macos.sh --identity "Apple Development: …"` (or
  `RF_SIGN_IDENTITY`) signs the app and the bundled `rf` with that identity
  instead of ad hoc, as TN3179 recommends. Ad hoc stays the default (the
  signing keychain is not usable over SSH), and the script says so.
- Not done, on purpose: running the daemon inside the app process or as a
  login item. The child process is already attributed to the app, and a
  login item or launchd agent would *lose* that attribution.

## Manual test (owner, on the Mac)

Requires the app built from this branch with `rf` bundled, and the LAN node
running.

1. Quit Ramflux Desktop. Replace `~/Applications/Ramflux Desktop.app` with the
   new build. Do not touch System Settings.
2. Open the app. On the first screen press **启动本地守护进程** (Start local
   daemon) and wait for it to answer.
3. On the node step enter `http://192.168.31.136:59482` and press **Look up**.
   - Expected: after up to a few seconds the node card appears (fingerprint,
     endpoints). No error. This is the case that failed before: the first
     lookup after installing a new build.
4. If macOS shows "Allow Ramflux Desktop to find devices on local networks?",
   choose **Allow**. If the card has not appeared by then, press **Look up**
   once more; expected: the card appears. No restart of the app or daemon.
5. Negative check: in System Settings › Privacy & Security › Local Network turn
   **Ramflux Desktop** off, look the node up again. Expected: after about 6 s
   the node step shows the Local Network explanation. Turn it back on and press
   **Look up**; expected: the card appears.
6. To see what macOS decided, run in Terminal while repeating step 3:
   `log stream --predicate 'eventMessage CONTAINS "LocalNetwork" OR eventMessage CONTAINS "allowed by preference" OR eventMessage CONTAINS "reason: NECP"'`.
   A `reason: NECP` drop followed by `allowed by preference … Clearing cached
   UUIDs` and then a successful card is the expected sequence after an update.
