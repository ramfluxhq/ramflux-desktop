#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (c) 2026 Span Brain
#
# Fails when a tracked Rust, shell or Python source lacks the repository's
# licence header (the SPDX line and the copyright line) in its first lines.
#
# Usage: scripts/check-license-headers.sh
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

missing=$(git ls-files '*.rs' '*.sh' '*.py' | while IFS= read -r file; do
  header=$(head -n 4 "$file")
  if ! printf '%s\n' "$header" | grep -Fq "SPDX-License-Identifier: AGPL-3.0-or-later" \
    || ! printf '%s\n' "$header" | grep -Fq "Copyright (c) 2026 Span Brain"; then
    echo "$file"
  fi
done)

if [ -n "$missing" ]; then
  echo "Files without the licence header (SPDX and copyright lines):"
  echo "$missing"
  exit 1
fi
echo "OK: every tracked .rs/.sh/.py file carries the licence header"
