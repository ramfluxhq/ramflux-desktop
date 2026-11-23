#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (c) 2026 Span Brain
#
# Builds the Linux release archive of the desktop client:
#
#   dist/ramflux-desktop-<version>-linux-<arch>.tar.gz
#   dist/SHA256SUMS
#
# Usage: scripts/package-linux.sh [--no-build] [--out DIR]
#
#   --no-build   package the existing target/release binary (built with
#                `--release --locked --no-default-features`, as this script
#                does).
#   --out DIR    write the archive and SHA256SUMS to DIR instead of dist/.
#
# Runs on Linux only (x86_64 or aarch64); the archive is for the host's
# architecture. It holds, under ramflux-desktop-<version>-linux-<arch>/:
#
#   README.md
#   usr/bin/ramflux-desktop
#   usr/share/applications/org.ramflux.desktop.desktop
#   usr/share/icons/hicolor/{48x48,64x64,128x128,256x256,512x512,scalable}/apps/
#   usr/share/metainfo/org.ramflux.desktop.metainfo.xml
#   usr/share/licenses/ramflux-desktop/
#
# Reproducibility:
#
# - The binary is built `--locked` without the `demo` feature, with build
#   paths remapped (`--remap-path-prefix`) so no local path is embedded, and
#   stripped of symbols; the GNU build id note is kept (`strip` keeps notes),
#   so a crash can still be matched to its build.
# - Every file time in the archive is SOURCE_DATE_EPOCH (default: the
#   commit time of HEAD); entries are sorted, owned by 0:0 with numeric ids,
#   and gzip stores no name or time. Two builds of the same commit with the
#   same toolchain and system libraries should give the same archive; that
#   is not yet verified (see docs/linux-verification.md).
# - The newest glibc symbol version the binary needs is printed: it bounds
#   the oldest distribution the archive runs on.

set -eu

APP_ID="org.ramflux.desktop"
EXECUTABLE="ramflux-desktop"
PACKAGE="ramflux-desktop"

die() {
    echo "package-linux: $*" >&2
    exit 1
}

[ "$(uname -s)" = "Linux" ] ||
    die "run this on Linux (this host is $(uname -s)); for macOS use scripts/package-macos.sh"

case "$(uname -m)" in
    x86_64 | amd64) ARCH="x86_64" ;;
    aarch64 | arm64) ARCH="aarch64" ;;
    *) die "unsupported architecture: $(uname -m) (x86_64 or aarch64)" ;;
esac

ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"

BUILD=1
OUT=""
while [ $# -gt 0 ]; do
    case "$1" in
        --no-build)
            BUILD=0
            shift
            ;;
        --out)
            [ $# -ge 2 ] || die "--out needs a directory"
            OUT="$2"
            shift 2
            ;;
        *) die "unknown argument: $1" ;;
    esac
done

for tool in cargo git tar gzip sha256sum strip; do
    command -v "$tool" >/dev/null 2>&1 || die "missing tool: $tool"
done

# One version source: the workspace version in Cargo.toml.
VERSION=$(cargo pkgid -p "$EXECUTABLE" | sed 's/.*[#@]//')
[ -n "$VERSION" ] || die "cannot read the version from Cargo"
REV=$(git rev-parse --short=12 HEAD 2>/dev/null || echo unknown)
if [ "$REV" != unknown ] && ! git diff --quiet HEAD -- 2>/dev/null; then
    REV="$REV-dirty"
fi
if [ -z "${SOURCE_DATE_EPOCH:-}" ]; then
    SOURCE_DATE_EPOCH=$(git log -1 --format=%ct 2>/dev/null || echo 0)
fi
export SOURCE_DATE_EPOCH

if [ "$BUILD" -eq 1 ]; then
    echo "building $EXECUTABLE $VERSION ($REV), release, linux-$ARCH"
    CARGO_HOME_DIR="${CARGO_HOME:-$HOME/.cargo}"
    # Without the default `demo` feature: no scenarios, mock source or
    # developer panel; the app always runs on a daemon.
    RAMFLUX_DESKTOP_BUILD_REV="$REV" \
        RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$ROOT=/build/ramflux-desktop --remap-path-prefix=$CARGO_HOME_DIR=/cargo" \
        cargo build --release --locked -p "$EXECUTABLE" --no-default-features
fi
BIN="target/release/$EXECUTABLE"
[ -x "$BIN" ] || die "missing $BIN (build first or drop --no-build)"

sh scripts/validate-linux-metadata.sh

DIST="${OUT:-$ROOT/dist}"
NAME="$PACKAGE-$VERSION-linux-$ARCH"
mkdir -p "$DIST"
WORK=$(mktemp -d "${TMPDIR:-/tmp}/rf-package.XXXXXX")
trap 'rm -rf "$WORK"' EXIT
STAGE="$WORK/$NAME"
SHARE="$STAGE/usr/share"

install -Dm755 "$BIN" "$STAGE/usr/bin/$EXECUTABLE"
strip --strip-all "$STAGE/usr/bin/$EXECUTABLE"
install -Dm644 "packaging/linux/$APP_ID.desktop" "$SHARE/applications/$APP_ID.desktop"
install -Dm644 "packaging/linux/$APP_ID.metainfo.xml" "$SHARE/metainfo/$APP_ID.metainfo.xml"
for size in 48x48 64x64 128x128 256x256 512x512; do
    install -Dm644 "packaging/linux/icons/hicolor/$size/apps/$APP_ID.png" \
        "$SHARE/icons/hicolor/$size/apps/$APP_ID.png"
done
install -Dm644 "packaging/linux/icons/hicolor/scalable/apps/$APP_ID.svg" \
    "$SHARE/icons/hicolor/scalable/apps/$APP_ID.svg"
for notice in LICENSE LICENSING.md NOTICE; do
    if [ -f "$notice" ]; then
        install -Dm644 "$notice" "$SHARE/licenses/$PACKAGE/$notice"
    fi
done
install -Dm644 packaging/linux/icons/LICENSE-LUCIDE "$SHARE/licenses/$PACKAGE/LICENSE-LUCIDE"
if [ -d licenses ]; then
    for file in licenses/*; do
        if [ -f "$file" ]; then
            install -Dm644 "$file" "$SHARE/licenses/$PACKAGE/$file"
        fi
    done
fi
install -Dm644 packaging/linux/README.md "$STAGE/README.md"

# The newest glibc symbol version the binary needs (informational).
if command -v objdump >/dev/null 2>&1; then
    glibc=$(objdump -T "$STAGE/usr/bin/$EXECUTABLE" 2>/dev/null |
        grep -o 'GLIBC_[0-9][0-9.]*' | sort -u -V | tail -1 || true)
    echo "package-linux: needs ${glibc:-an unknown glibc version} or newer"
fi

ARCHIVE="$NAME.tar.gz"
find "$STAGE" -exec touch -h -d "@$SOURCE_DATE_EPOCH" {} +
(cd "$WORK" && tar --sort=name --mtime="@$SOURCE_DATE_EPOCH" \
    --owner=0 --group=0 --numeric-owner \
    --pax-option=exthdr.name=%d/PaxHeaders/%f,delete=atime,delete=ctime \
    -cf - "$NAME") | gzip -9 -n >"$DIST/$ARCHIVE"

# SHA256SUMS covers every archive of this version in the output folder (the
# release workflow gathers both architectures into one folder).
(cd "$DIST" && sha256sum "$PACKAGE-$VERSION"-linux-*.tar.gz >SHA256SUMS)

echo "built: $DIST/$ARCHIVE"
echo "version: $VERSION ($REV), app id: $APP_ID, SOURCE_DATE_EPOCH: $SOURCE_DATE_EPOCH"
cat "$DIST/SHA256SUMS"
