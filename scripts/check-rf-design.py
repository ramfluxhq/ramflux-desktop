#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (c) 2026 Span Brain
"""Verify imported RF artifacts against their reviewed design-pipeline manifest.

Every theme is a token file in design/generated/ (the desktop embeds each one at
build time). Light and dark are required; any further theme file must be listed
in the manifest like the others, so it comes from the reviewed pipeline.
"""
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
REQUIRED = (
    "design/tokens/rf-tokens.json",
    "design/generated/light.json",
    "design/generated/dark.json",
    "crates/ramflux-desktop-navigation/src/geometry.rs",
)


def main():
    manifest = json.loads((ROOT / "design/generated/manifest.json").read_text(encoding="utf-8"))
    listed = set(manifest.get("files", {}))
    themes = {
        f"design/generated/{path.name}"
        for path in (ROOT / "design/generated").glob("*.json")
        if path.name != "manifest.json"
    }
    if manifest.get("format_version") != 1 or not set(REQUIRED) <= listed:
        raise SystemExit("Invalid RF design import manifest")
    if not themes <= listed:
        raise SystemExit(f"Theme token files not in the manifest: {sorted(themes - listed)}")
    if not listed <= set(REQUIRED) | themes:
        raise SystemExit(f"Manifest lists unknown files: {sorted(listed - set(REQUIRED) - themes)}")
    for name in sorted(listed):
        path = ROOT / name
        if not path.is_file() or hashlib.sha256(path.read_bytes()).hexdigest() != manifest["files"][name]:
            raise SystemExit(f"Stale RF design artifact: {name}")
    generated = ROOT / "crates/ramflux-desktop-navigation/src/generated"
    imported = json.loads((generated / ".pen-gen-manifest.json").read_text(encoding="utf-8"))
    if (imported.get("owner") != "ramflux design generator sync desktop"
            or imported.get("generator") != "ramflux design generator"
            or imported.get("version") != 1):
        raise SystemExit("Invalid RF component import manifest")
    for name, digest in imported.get("files", {}).items():
        path = generated / name
        if path.is_symlink() or not path.resolve().is_relative_to(generated.resolve()):
            raise SystemExit("Invalid RF component import path")
        if not path.is_file() or hashlib.sha256(path.read_bytes()).hexdigest() != digest:
            raise SystemExit(f"Stale RF component artifact: {name}")
    print("RF_DESIGN_OK")


if __name__ == "__main__":
    main()
