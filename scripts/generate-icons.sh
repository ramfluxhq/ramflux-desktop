#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (c) 2026 Span Brain
#
# Regenerates every platform icon from the two brand sources:
#
#   packaging/brand/ramflux-mark.svg           full-colour app icon
#   packaging/brand/ramflux-mark-symbolic.svg  single-colour symbolic icon
#
# Outputs (all committed, so packaging needs no generator at build time):
#
#   packaging/linux/icons/hicolor/{16,24,32,48,64,128,256,512}/apps/org.ramflux.desktop.png
#   packaging/linux/icons/hicolor/scalable/apps/org.ramflux.desktop.svg
#   packaging/linux/icons/hicolor/symbolic/apps/org.ramflux.desktop-symbolic.svg
#   packaging/macos/AppIcon.icns            (1x and @2x, 16 to 512 pt)
#   packaging/windows/ramflux.ico           (16, 24, 32, 48, 64, 128, 256)
#
# Usage: scripts/generate-icons.sh
#
# Tools (macOS, nothing to install): `sips` renders the SVG to a PNG at each
# size directly from the vector, `iconutil` assembles the .icns, and python3
# (standard library only) assembles the .ico from PNG entries. The macOS
# icon keeps the template padding of the source; the Linux icons use the
# same artwork with the viewBox cropped so the tile fills more of the canvas.

set -eu

APP_ID="org.ramflux.desktop"
# Linux viewBox: the 824 px tile (at 100..924) with a 32 px margin.
LINUX_VIEWBOX="68 68 888 888"

die() {
    echo "generate-icons: $*" >&2
    exit 1
}

[ "$(uname -s)" = "Darwin" ] ||
    die "run this on macOS (needs sips and iconutil; no other renderer is supported)"
for tool in sips iconutil python3; do
    command -v "$tool" >/dev/null 2>&1 || die "missing tool: $tool"
done

ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
MARK="packaging/brand/ramflux-mark.svg"
SYMBOLIC="packaging/brand/ramflux-mark-symbolic.svg"
[ -f "$MARK" ] || die "missing $MARK"
[ -f "$SYMBOLIC" ] || die "missing $SYMBOLIC"

WORK=$(mktemp -d "${TMPDIR:-/tmp}/rf-icons.XXXXXX")
trap 'rm -rf "$WORK"' EXIT

# render SVG SIZE OUT: rasterise the vector at SIZE x SIZE pixels.
render() {
    sips -s format png -z "$2" "$2" "$1" --out "$3" >/dev/null 2>&1 ||
        die "sips failed to render $1 at $2 px"
    got=$(sips -g pixelWidth "$3" | sed -n 's/.*pixelWidth: //p')
    [ "$got" = "$2" ] || die "sips produced $got px instead of $2 px for $1"
}

echo "generate-icons: renderer: sips (vector render per size), iconutil, python3 struct for .ico"

# Linux: cropped viewBox, hicolor PNGs, scalable and symbolic SVGs.
LINUX_SVG="$WORK/linux.svg"
sed "s|viewBox=\"0 0 1024 1024\"|viewBox=\"$LINUX_VIEWBOX\"|" "$MARK" >"$LINUX_SVG"
grep -q "viewBox=\"$LINUX_VIEWBOX\"" "$LINUX_SVG" || die "cannot set the Linux viewBox"
ICONS="packaging/linux/icons/hicolor"
for size in 16 24 32 48 64 128 256 512; do
    mkdir -p "$ICONS/${size}x${size}/apps"
    render "$LINUX_SVG" "$size" "$ICONS/${size}x${size}/apps/$APP_ID.png"
done
mkdir -p "$ICONS/scalable/apps" "$ICONS/symbolic/apps"
cp "$LINUX_SVG" "$ICONS/scalable/apps/$APP_ID.svg"
cp "$SYMBOLIC" "$ICONS/symbolic/apps/$APP_ID-symbolic.svg"

# macOS: the padded source, iconset sizes 16, 32, 128, 256, 512 at 1x and 2x.
SET="$WORK/AppIcon.iconset"
mkdir -p "$SET" packaging/macos
for size in 16 32 128 256 512; do
    render "$MARK" "$size" "$SET/icon_${size}x${size}.png"
    render "$MARK" $((size * 2)) "$SET/icon_${size}x${size}@2x.png"
done
iconutil -c icns "$SET" -o packaging/macos/AppIcon.icns || die "iconutil failed"

# Windows: the Linux artwork (fuller tile reads better at 16 px) as an .ico
# with PNG-compressed entries.
mkdir -p "$WORK/ico" packaging/windows
for size in 16 24 32 48 64 128 256; do
    render "$LINUX_SVG" "$size" "$WORK/ico/$size.png"
done
python3 -I - "$WORK/ico" packaging/windows/ramflux.ico <<'PY'
import struct
import sys

src, out = sys.argv[1], sys.argv[2]
sizes = [16, 24, 32, 48, 64, 128, 256]
images = [open(f"{src}/{s}.png", "rb").read() for s in sizes]
head = struct.pack("<HHH", 0, 1, len(sizes))
offset = 6 + 16 * len(sizes)
entries = b""
for s, data in zip(sizes, images):
    dim = 0 if s == 256 else s
    entries += struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(data), offset)
    offset += len(data)
with open(out, "wb") as f:
    f.write(head + entries + b"".join(images))
PY

echo "generate-icons: done"
