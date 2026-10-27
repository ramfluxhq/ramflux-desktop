#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (c) 2026 Span Brain
#
# Lints every screenshot and rehearsal program of the UI crate.
#
# Those programs are declared with `test = false` (they are run explicitly,
# not by `cargo test`), and cargo's `--all-targets` skips such targets, so a
# change that breaks one would go unnoticed. This script asks cargo itself
# (`cargo metadata`) which test targets of the UI crate are switched off, so
# a new program is covered without editing the script, and runs clippy on
# exactly those targets. It fails loudly when the metadata cannot be read or
# names no program.
#
# Usage: scripts/check-shot-programs.sh
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

if ! command -v python3 >/dev/null 2>&1; then
  echo "check-shot-programs: python3 is required to read cargo metadata" >&2
  exit 1
fi

metadata=$(cargo metadata --no-deps --format-version 1) || {
  echo "check-shot-programs: cargo metadata failed" >&2
  exit 1
}

names=$(printf '%s' "$metadata" | python3 -c '
import json, sys
try:
    data = json.load(sys.stdin)
    package = next(p for p in data["packages"] if p["name"] == "ramflux-desktop-ui")
    names = [t["name"] for t in package["targets"]
             if t["kind"] == ["test"] and t.get("test") is False]
except (ValueError, KeyError, StopIteration) as error:
    sys.exit("check-shot-programs: cannot read cargo metadata: %r" % (error,))
print("\n".join(sorted(names)))
') || exit 1

if [ -z "$names" ]; then
  echo "check-shot-programs: no screenshot programs (test = false) in ramflux-desktop-ui" >&2
  exit 1
fi

set --
for name in $names; do
  set -- "$@" --test "$name"
done
echo "linting $(echo "$names" | wc -l | tr -d ' ') screenshot programs"
cargo clippy -p ramflux-desktop-ui --all-features "$@" -- -D warnings
