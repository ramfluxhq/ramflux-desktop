#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (c) 2026 Span Brain
#
# Copy the card host fixtures and the bundled card trust package from a core
# checkout at a pinned revision into `card-fixtures/`, and write
# `card-fixtures/MANIFEST.json` with a BLAKE3 hash of every copied file.
#
# Usage:
#   scripts/sync-card-fixtures.sh <core-checkout> <full-core-rev>
#   scripts/sync-card-fixtures.sh <core-checkout> <full-core-rev> --check
#
# The copies are never edited. A sync refuses when a local copy no longer
# matches the manifest (drift). `--check` refuses when the checkout is not at
# the revision, when a copy differs from the checkout, when a hash differs
# from the manifest, when a file is extra or missing, or when the desktop's
# pinned `ramflux-sdk` revision differs from the manifest.
# Needs python3 with the `blake3` module.

set -eu

if [ "$#" -lt 2 ] || [ "$#" -gt 3 ]; then
    echo "usage: $0 <core-checkout> <full-core-rev> [--check]" >&2
    exit 2
fi
mode="sync"
if [ "$#" -eq 3 ]; then
    if [ "$3" != "--check" ]; then
        echo "usage: $0 <core-checkout> <full-core-rev> [--check]" >&2
        exit 2
    fi
    mode="check"
fi

repo_root=$(cd "$(dirname "$0")/.." && pwd)

python3 - "$1" "$2" "$mode" "$repo_root" <<'PY'
import glob
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys

try:
    import blake3
except ImportError:
    sys.exit("sync-card-fixtures: python3 module `blake3` is required")

core, rev, mode, repo = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4]
DEST = os.path.join(repo, "card-fixtures")
MANIFEST = os.path.join(DEST, "MANIFEST.json")
ACTIONS = ["approval", "chart", "complete_form", "form", "task"]
SUFFIXES = ["json", "mount", "edit", "updated", "tree"]
TRUST = "crates/ramflux-sdk/include/card/trust.json"


def fail(message):
    sys.exit("sync-card-fixtures: " + message)


def digest(path):
    with open(path, "rb") as handle:
        return blake3.blake3(handle.read()).hexdigest()


if not re.fullmatch(r"[0-9a-f]{40}", rev):
    fail("the core revision must be a full 40-character commit id")
head = subprocess.run(["git", "-C", core, "rev-parse", "HEAD"], capture_output=True, text=True)
if head.returncode != 0 or head.stdout.strip() != rev:
    fail("core checkout is not at " + rev)

# Source files, relative to the core checkout, mapped to their copy paths.
sources = {}
for root, _, names in os.walk(os.path.join(core, "crates/ramflux-card/host-fixtures")):
    for name in names:
        full = os.path.join(root, name)
        rel = os.path.relpath(full, os.path.join(core, "crates/ramflux-card"))
        sources[rel.replace(os.sep, "/")] = full
for action in ACTIONS:
    for suffix in SUFFIXES:
        name = "fixtures/%s.json" % action if suffix == "json" else "fixtures/%s.%s.json" % (action, suffix)
        source = os.path.join(core, "crates/ramflux-card", name)
        if not os.path.isfile(source):
            fail("missing source fixture " + name)
        sources[name] = source
sources["trust.json"] = os.path.join(core, TRUST)
if not os.path.isfile(sources["trust.json"]):
    fail("missing trust package " + TRUST)

present = set()
for root, _, names in os.walk(DEST) if os.path.isdir(DEST) else []:
    for name in names:
        present.add(os.path.relpath(os.path.join(root, name), DEST).replace(os.sep, "/"))

manifest = None
if os.path.isfile(MANIFEST):
    with open(MANIFEST, encoding="utf-8") as handle:
        manifest = json.load(handle)
    for row in manifest["files"]:
        path = os.path.join(DEST, row["path"])
        if not os.path.isfile(path) or digest(path) != row["blake3"]:
            fail("local copy drifted from MANIFEST.json: " + row["path"])

if mode == "sync":
    for copy, source in sorted(sources.items()):
        target = os.path.join(DEST, copy)
        os.makedirs(os.path.dirname(target), exist_ok=True)
        shutil.copyfile(source, target)
    for stale in sorted(present - set(sources) - {"MANIFEST.json"}):
        os.remove(os.path.join(DEST, stale))
    files = [{"path": copy, "blake3": digest(os.path.join(DEST, copy))} for copy in sorted(sources)]
    with open(MANIFEST, "w", encoding="utf-8") as handle:
        json.dump({"core_rev": rev, "files": files}, handle, indent=2)
        handle.write("\n")
    print("sync-card-fixtures: copied %d files at %s" % (len(files), rev))
    sys.exit(0)

# --check
problems = []
if manifest is None:
    problems.append("MANIFEST.json is missing")
else:
    if manifest.get("core_rev") != rev:
        problems.append("MANIFEST core_rev differs from " + rev)
    listed = {row["path"]: row["blake3"] for row in manifest["files"]}
    for copy, source in sorted(sources.items()):
        if copy not in listed:
            problems.append("not in MANIFEST: " + copy)
        elif digest(source) != listed[copy]:
            problems.append("checkout differs from MANIFEST: " + copy)
    for copy in sorted(set(listed) - set(sources)):
        problems.append("MANIFEST lists a file the checkout no longer has: " + copy)
    for copy in sorted(present - set(listed) - {"MANIFEST.json"}):
        problems.append("extra file in card-fixtures: " + copy)
cargo = os.path.join(repo, "Cargo.toml")
with open(cargo, encoding="utf-8") as handle:
    pins = set(re.findall(r'ramflux-sdk = \{[^}]*rev = "([0-9a-f]{40})"', handle.read()))
if pins != {rev}:
    problems.append("desktop pins ramflux-sdk at %s, expected %s" % (sorted(pins), rev))
if problems:
    for line in problems:
        print("sync-card-fixtures: " + line, file=sys.stderr)
    sys.exit(1)
print("sync-card-fixtures: %d files match %s" % (len(sources), rev))
PY
