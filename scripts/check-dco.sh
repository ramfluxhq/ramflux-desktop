#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (c) 2026 Span Brain
#
# Checks the Developer Certificate of Origin sign-off of new commits: every
# non-merge commit in BASE..HEAD must carry a `Signed-off-by:` trailer that
# names its author (`git commit -s`). History before BASE is not checked, and
# commits by GitHub bot accounts (dependency updates) are exempt.
#
# Usage: scripts/check-dco.sh BASE [HEAD]
set -eu

if [ $# -lt 1 ] || [ $# -gt 2 ]; then
  echo "usage: scripts/check-dco.sh BASE [HEAD]" >&2
  exit 2
fi
base=$1
head=${2:-HEAD}

commits=$(git rev-list --no-merges "$base..$head") || {
  echo "check-dco: cannot list $base..$head" >&2
  exit 2
}

failed=0
for commit in $commits; do
  author=$(git log -1 --format='%an <%ae>' "$commit")
  case $author in
    *"[bot]@users.noreply.github.com>") continue ;;
  esac
  if ! git log -1 --format='%(trailers:key=Signed-off-by,valueonly)' "$commit" \
    | grep -Fqx "$author"; then
    echo "missing sign-off by $author: $(git log -1 --format='%h %s' "$commit")"
    failed=1
  fi
done

if [ "$failed" -ne 0 ]; then
  echo "Sign off each commit with 'git commit -s' (see CONTRIBUTING.md)."
  exit 1
fi
echo "OK: every new commit is signed off by its author"
