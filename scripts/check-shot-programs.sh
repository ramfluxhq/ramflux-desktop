#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (c) 2026 Span Brain
#
# Lints every screenshot and rehearsal program of the UI crate.
#
# Those programs are declared with `test = false` (they are run explicitly,
# not by `cargo test`), and cargo's `--all-targets` skips such targets, so a
# change that breaks one would go unnoticed. This script reads them from the
# crate manifest, so a new program is covered without editing the script, and
# runs clippy on exactly those targets.
#
# Usage: scripts/check-shot-programs.sh
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
manifest="$root/crates/ramflux-desktop-ui/Cargo.toml"

names=$(awk '
  /^\[\[test\]\]/ { if (name != "" && off) print name; name = ""; off = 0; next }
  /^\[/ { if (name != "" && off) print name; name = ""; off = 0 }
  /^name *= *"/ { split($0, p, "\""); name = p[2] }
  /^test *= *false/ { off = 1 }
  END { if (name != "" && off) print name }
' "$manifest")

if [ -z "$names" ]; then
  echo "no screenshot programs found in $manifest" >&2
  exit 1
fi

set --
for name in $names; do
  set -- "$@" --test "$name"
done
echo "linting $(echo "$names" | wc -l | tr -d ' ') screenshot programs"
cd "$root"
cargo clippy -p ramflux-desktop-ui --all-features "$@" -- -D warnings
