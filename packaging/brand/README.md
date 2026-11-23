# Ramflux brand mark

The application icon is the Ramflux mark ("Flux Current") taken from the
project website, <https://ramflux.org> (its favicon and the header mark): an
ink S-curve from a vermilion filled dot on the left to an ink ring on the
right. The geometry is the website's, unchanged.

| Colour | Hex |
|---|---|
| Paper | `#faf8f3` |
| Ink | `#0a0a0a` |
| Vermilion | `#b43a2f` |

Sources:

- `ramflux-mark.svg`: the full-colour icon on a 1024 grid, a paper rounded
  tile inset 100 px (the macOS template). The Linux and Windows icons use the
  same artwork with a tighter viewBox.
- `ramflux-mark-symbolic.svg`: the single-colour symbolic icon with no
  background (freedesktop convention).

Every platform icon (`packaging/linux/icons/`, `packaging/macos/`,
`packaging/windows/`) is generated from these two files by
`scripts/generate-icons.sh`. Edit the sources and rerun the script; do not
edit the generated files by hand.
