# Ramflux Desktop for Linux

This archive holds the Ramflux desktop client, built for release without the
demo scenarios, and its desktop integration files:

```text
usr/bin/ramflux-desktop
usr/share/applications/org.ramflux.desktop.desktop
usr/share/icons/hicolor/<size>/apps/org.ramflux.desktop.{png,svg}
usr/share/icons/hicolor/symbolic/apps/org.ramflux.desktop-symbolic.svg
usr/share/metainfo/org.ramflux.desktop.metainfo.xml
usr/share/licenses/ramflux-desktop/
```

## Install

System-wide (as root), then refresh the caches your desktop uses:

```sh
cp -r usr/. /usr/
gtk-update-icon-cache -q -t -f /usr/share/icons/hicolor || true
update-desktop-database -q /usr/share/applications || true
```

For one user, without root:

```sh
install -Dm755 usr/bin/ramflux-desktop ~/.local/bin/ramflux-desktop
cp -r usr/share/. ~/.local/share/
```

On Arch Linux and Omarchy, prefer the `ramflux-desktop-bin` package, which
installs the same files and removes them cleanly.

## Run

The desktop client talks to a Ramflux daemon (`rf daemon start`) on the same
computer. Install `rf` from the Ramflux CLI release; when `rf` sits next to
`ramflux-desktop` (for example both in `/usr/bin`), "Start local daemon" on
the first screen finds it without any setting.

More: `docs/linux.md` in the source repository,
<https://github.com/ramfluxhq/ramflux-desktop>.

## Licence

AGPL-3.0-or-later; see `usr/share/licenses/ramflux-desktop/`. The icon is the
Ramflux mark from <https://ramflux.org>.
