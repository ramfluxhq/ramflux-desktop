#!/usr/bin/env python3
"""Generate GPUI themes and geometry from the shared W3C token snapshot."""
import argparse
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TOKENS = ROOT / "design/tokens/rf-tokens.json"
OUTPUT = ROOT / "design/generated"
ROLES = {
    "background": "bg", "foreground": "text", "sidebar": "nav",
    "sidebar_foreground": "text", "sidebar_border": "line-nav",
    "sidebar_accent": "selected", "sidebar_accent_foreground": "text",
    "border": "line", "input": "input-border", "ring": "focus",
    "primary": "primary", "primary_foreground": "on-primary",
    "primary_hover": "accent", "primary_active": "accent",
    "secondary": "surface", "secondary_foreground": "text",
    "secondary_hover": "selected", "secondary_active": "selected",
    "muted": "surface", "muted_foreground": "secondary",
    "accent": "selected", "accent_foreground": "text",
    "popover": "overlay", "popover_foreground": "text",
    "group_box": "surface-white", "group_box_foreground": "text",
    "title_bar": "bg", "title_bar_border": "line",
    "selection": "selected", "list_active": "selected",
    "list_active_border": "focus", "list_hover": "surface",
    "danger": "danger", "danger_foreground": "on-danger",
    "warning": "warning", "success": "success",
}


def color(token):
    value = token["$value"]
    channels = value["components"] + [value["alpha"]]
    return "#" + "".join(f"{round(c * 255):02X}" for c in channels)


def outputs():
    tokens = json.loads(TOKENS.read_text())
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
            if not path.exists() or path.read_text() != content:
                raise SystemExit(f"Stale generated tokens: {path.relative_to(ROOT)}")
        else:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content)
    print("RF_TOKENS_OK")


if __name__ == "__main__":
    main()
