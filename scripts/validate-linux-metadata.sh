#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (c) 2026 Span Brain
#
# Validates the Linux desktop integration files in packaging/linux/:
#
#   org.ramflux.desktop.desktop        desktop-file-validate
#   org.ramflux.desktop.metainfo.xml   appstreamcli validate --no-net --strict
#   icons/hicolor/...                  presence of every size
#
# Usage: scripts/validate-linux-metadata.sh [DIR]
#
#   DIR   a directory laid out like packaging/linux (the default).
#
# A validator that is not installed is skipped with a message, so the
# script runs anywhere; set REQUIRE_VALIDATORS=1 (CI does) to fail instead.
# RAMFLUX_METAINFO_STRICT=0 drops --strict from the AppStream check.

set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
DIR="${1:-$ROOT/packaging/linux}"
APP_ID="org.ramflux.desktop"
REQUIRE="${REQUIRE_VALIDATORS:-0}"
STRICT="${RAMFLUX_METAINFO_STRICT:-1}"

failed=0
fail() {
    echo "validate-linux-metadata: FAIL: $*" >&2
    failed=1
}
skip() {
    if [ "$REQUIRE" = 1 ]; then
        fail "$1 is not installed (REQUIRE_VALIDATORS=1)"
    else
        echo "validate-linux-metadata: skipped: $1 is not installed"
    fi
}

entry="$DIR/$APP_ID.desktop"
metainfo="$DIR/$APP_ID.metainfo.xml"
[ -f "$entry" ] || fail "missing $entry"
[ -f "$metainfo" ] || fail "missing $metainfo"

for size in 16x16 24x24 32x32 48x48 64x64 128x128 256x256 512x512; do
    [ -f "$DIR/icons/hicolor/$size/apps/$APP_ID.png" ] || fail "missing $size icon"
done
[ -f "$DIR/icons/hicolor/scalable/apps/$APP_ID.svg" ] || fail "missing scalable icon"
[ -f "$DIR/icons/hicolor/symbolic/apps/$APP_ID-symbolic.svg" ] || fail "missing symbolic icon"

if [ -f "$entry" ]; then
    grep -qx "StartupWMClass=$APP_ID" "$entry" || fail "StartupWMClass is not $APP_ID"
    grep -qx "Icon=$APP_ID" "$entry" || fail "Icon is not $APP_ID"
    if command -v desktop-file-validate >/dev/null 2>&1; then
        if desktop-file-validate "$entry"; then
            echo "validate-linux-metadata: ok: $(basename "$entry")"
        else
            fail "desktop-file-validate $(basename "$entry")"
        fi
    else
        skip desktop-file-validate
    fi
fi

if [ -f "$metainfo" ]; then
    if command -v appstreamcli >/dev/null 2>&1; then
        if [ "$STRICT" = 1 ]; then
            set -- --no-net --strict --explain
        else
            set -- --no-net --explain
        fi
        if appstreamcli validate "$@" "$metainfo"; then
            echo "validate-linux-metadata: ok: $(basename "$metainfo")"
        else
            fail "appstreamcli validate $(basename "$metainfo")"
        fi
    else
        skip appstreamcli
    fi
fi

[ "$failed" -eq 0 ] || exit 1
echo "validate-linux-metadata: done"
