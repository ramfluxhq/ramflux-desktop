#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (c) 2026 Span Brain
"""Verify imported RF artifacts against their reviewed design-pipeline manifest."""
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FILES = (
    "design/tokens/rf-tokens.json",
    "design/generated/light.json",
    "design/generated/dark.json",
    "crates/ramflux-desktop-navigation/src/geometry.rs",
)


def main():
    manifest = json.loads((ROOT / "design/generated/manifest.json").read_text(encoding="utf-8"))
    if manifest.get("format_version") != 1 or set(manifest.get("files", {})) != set(FILES):
        raise SystemExit("Invalid RF design import manifest")
    for name in FILES:
        path = ROOT / name
        if not path.is_file() or hashlib.sha256(path.read_bytes()).hexdigest() != manifest["files"][name]:
            raise SystemExit(f"Stale RF design artifact: {name}")
    print("RF_DESIGN_OK")


if __name__ == "__main__":
    main()
