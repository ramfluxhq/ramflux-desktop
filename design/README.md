# RF design artifacts

The token snapshot, light/dark GPUI themes and navigation geometry are imported
outputs of the design compiler. Their hashes are recorded in `generated/manifest.json` and
checked by `python3 scripts/check-rf-design.py` in CI. Update these artifacts
together from the reviewed design pipeline; do not maintain another token
generator or edit the snapshot as an independent source.

The native theme adapter loads both registered GPUI Kit themes and the
RF-specific rail/avatar roles. Navigation and state stay handwritten. The first
generated conversation-row probe uses the mobile reference geometry; the desktop
adapter follows its desktop reference's 64-pixel rows and 40-pixel avatars.

The presentation catalog is imported into the navigation crate's `generated/`
directory by the design compiler's desktop sync command. Its ownership manifest
records every generated file, including the native themes; the design guard
checks those hashes. Generated modules are excluded from rustfmt so imports
remain byte-identical. Conversation behavior, native editors, selection, menus
and mutation availability remain handwritten by their feature owners.

## Themes are token sets

Every `generated/<id>.json` except `manifest.json` is a theme: the build of the
navigation crate embeds each one and registers it, so adding a theme needs no
code change. Its RF roles (rail, avatars, bubbles, radii, type sizes) come from
`tokens/rf-tokens.json` under the key `<id>`, or under the theme's mode
(`light`/`dark`) when the token file has no such key; a token the set leaves out
keeps the mode's generated value. To add one, export `<id>.json` (and optionally
the `<id>` token set) from the design pipeline, record both in the manifest, and
optionally name it in the locale files as `theme.name.<id>`. The Settings picker
lists it next to Light, Dark and Follow system.
