#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (c) 2026 Span Brain
"""Generate GPUI themes and geometry from the shared W3C token snapshot."""
import argparse
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TOKENS = ROOT / "design/tokens/rf-tokens.json"
OUTPUT = ROOT / "design/generated"
ROLES = {
    "background": "bg", "foreground": "text", "sidebar.background": "nav",
    "sidebar.foreground": "text", "sidebar.border": "line-nav",
    "sidebar.accent.background": "selected", "sidebar.accent.foreground": "text",
    "border": "line", "input.border": "input-border", "ring": "focus",
    "primary.background": "primary", "primary.foreground": "on-primary",
    "primary.hover.background": "accent", "primary.active.background": "accent",
    "secondary.background": "surface", "secondary.foreground": "text",
    "secondary.hover.background": "selected", "secondary.active.background": "selected",
    "muted.background": "surface", "muted.foreground": "secondary",
    "accent.background": "selected", "accent.foreground": "text",
    "popover.background": "overlay", "popover.foreground": "text",
    "group_box.background": "surface-white", "group_box.foreground": "text",
    "title_bar.background": "bg", "title_bar.border": "line",
    "selection.background": "selected", "list.active.background": "selected",
    "list.active.border": "focus", "list.hover.background": "surface",
    "danger.background": "danger", "danger.foreground": "on-danger",
    "warning.background": "warning", "success.background": "success",
}


def color(token):
    value = token["$value"]
    channels = value["components"] + [value["alpha"]]
    return "#" + "".join(f"{round(c * 255):02X}" for c in channels)


def outputs():
    tokens = json.loads(TOKENS.read_text(encoding="utf-8"))
    files = {}
    for mode in ("light", "dark"):
        values = tokens[mode]
        theme = {
            "name": f"Ramflux {mode.title()}", "mode": mode,
            "font.family": values["font-latin"]["$value"],
            "mono_font.family": values["font-mono"]["$value"],
            "font.size": values["size-body"]["$value"]["value"],
            "radius": values["radius-button"]["$value"]["value"],
            "radius.lg": values["radius-dialog"]["$value"]["value"],
            "colors": {role: color(values[name]) for role, name in ROLES.items()},
        }
        files[f"{mode}.json"] = json.dumps(theme, indent=2) + "\n"
    return files


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    for name, content in outputs().items():
        path = OUTPUT / name
        if args.check:
            if not path.exists() or path.read_text(encoding="utf-8") != content:
                raise SystemExit(f"Stale generated tokens: {path.relative_to(ROOT)}")
        else:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content, encoding="utf-8")
    print("RF_TOKENS_OK")


if __name__ == "__main__":
    main()
