# RF design artifacts

The token snapshot, light/dark GPUI themes and navigation geometry are imported
outputs of PEN-GEN-01. Their hashes are recorded in `generated/manifest.json` and
checked by `python3 scripts/check-rf-design.py` in CI. Update these artifacts
together from the reviewed design pipeline; do not maintain another token
generator or edit the snapshot as an independent source.

The native theme adapter loads both registered GPUI Kit themes and the
RF-specific rail/avatar roles. Navigation and state stay handwritten. The first
generated conversation-row probe uses the mobile MOB-B01 geometry; the desktop
adapter follows its desktop reference's 64-pixel rows and 40-pixel avatars.
