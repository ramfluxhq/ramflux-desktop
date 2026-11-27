#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (c) 2026 Span Brain
#
# Builds a macOS application bundle for local use and demos:
#
#   dist/Ramflux Desktop.app
#
# Usage: scripts/package-macos.sh [--rf PATH] [--no-build] [--no-sign]
#                                 [--identity NAME] [--out DIR]
#
#   --rf PATH    also ship this `rf` binary inside the bundle (next to the
#                desktop executable), so "Start local daemon" works without
#                configuring a program path. RF_BIN=PATH does the same.
#                Cards need the daemon's card runtime: build it with
#                `cargo build -p ramflux-rf --profile ffi-release` in the core
#                checkout.
#   --no-build   package the existing target/release binary (build it with
#                `--no-default-features`, as this script does).
#   --no-sign    leave the bundle unsigned (default: ad hoc signature, which
#                is enough to run the bundle on the machine that built it).
#   --identity NAME
#                sign with this code-signing identity (an Apple Development
#                or Developer ID certificate in the keychain) instead of ad
#                hoc. RF_SIGN_IDENTITY=NAME does the same. macOS privacy
#                settings such as Local Network follow an app by its code
#                signature, and only an Apple-issued identity keeps that
#                identity stable from one build to the next (TN3179).
#   --out DIR    write the bundle to DIR instead of dist/.
#
# The script refuses a binary whose dependency graph contains the card
# runtime crate or QuickJS, or that carries card gallery or QuickJS symbols
# (`card-gallery` is a test-build feature only).
#
# The result is a development build: it is not signed with a Developer ID
# and not notarized, so it is not ready for distribution. See docs/demo.md.
#
# The bundle holds only the executable, an optional `rf`, Info.plist, the
# icon and license notices; no settings, keys or account data. User data
# lives outside the bundle (see docs/demo.md), so replacing or deleting the
# bundle never touches it.

set -eu

APP_NAME="Ramflux Desktop"
BUNDLE_ID="org.ramflux.desktop"
EXECUTABLE="ramflux-desktop"
MIN_MACOS="15.0"

die() {
    echo "package-macos: $*" >&2
    exit 1
}

[ "$(uname -s)" = "Darwin" ] || die "run this on macOS"

ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"

RF="${RF_BIN:-}"
BUILD=1
SIGN=1
IDENTITY="${RF_SIGN_IDENTITY:-}"
OUT=""
while [ $# -gt 0 ]; do
    case "$1" in
        --rf)
            [ $# -ge 2 ] || die "--rf needs a path"
            RF="$2"
            shift 2
            ;;
        --no-build)
            BUILD=0
            shift
            ;;
        --no-sign)
            SIGN=0
            shift
            ;;
        --identity)
            [ $# -ge 2 ] || die "--identity needs a name"
            IDENTITY="$2"
            shift 2
            ;;
        --out)
            [ $# -ge 2 ] || die "--out needs a directory"
            OUT="$2"
            shift 2
            ;;
        *) die "unknown argument: $1" ;;
    esac
done

# One version source: the workspace version in Cargo.toml.
VERSION=$(cargo pkgid -p "$EXECUTABLE" | sed 's/.*[#@]//')
[ -n "$VERSION" ] || die "cannot read the version from Cargo"
REV=$(git rev-parse --short=12 HEAD 2>/dev/null || echo unknown)
if [ "$REV" != unknown ] && ! git diff --quiet HEAD -- 2>/dev/null; then
    REV="$REV-dirty"
fi

if [ "$BUILD" -eq 1 ]; then
    echo "building $EXECUTABLE $VERSION ($REV), release"
    # Without the default `demo` feature: no scenarios, mock source or
    # developer panel in the bundle; the app always runs on a daemon.
    RAMFLUX_DESKTOP_BUILD_REV="$REV" cargo build --release --locked -p "$EXECUTABLE" \
        --no-default-features
fi
BIN="target/release/$EXECUTABLE"
[ -x "$BIN" ] || die "missing $BIN (build first or drop --no-build)"

# The card gallery, its conformance and the standalone card runtime
# (QuickJS) are test-build only: the release graph must not pull them in,
# and the binary must not carry their symbols.
tree=$(cargo tree --locked -e normal -p "$EXECUTABLE" --no-default-features --prefix none 2>/dev/null) \
    || die "cannot read the release dependency graph"
if printf '%s\n' "$tree" | grep -E -q '^(ramflux-card|rquickjs[a-z-]*) '; then
    die "the release dependency graph contains ramflux-card or rquickjs (card-gallery must stay off)"
fi
symbols=$(nm -U "$BIN" 2>/dev/null) || die "cannot list the symbols of $BIN"
if printf '%s\n' "$symbols" | grep -E -i -q 'card_gallery|rquickjs|quickjs|JS_NewRuntime'; then
    die "$BIN carries card gallery or QuickJS symbols (card-gallery must stay off)"
fi

DIST="${OUT:-$ROOT/dist}"
APP="$DIST/$APP_NAME.app"
CONTENTS="$APP/Contents"
rm -rf "$APP"
mkdir -p "$CONTENTS/MacOS" "$CONTENTS/Resources/licenses"
cp "$BIN" "$CONTENTS/MacOS/$EXECUTABLE"

if [ -n "$RF" ]; then
    [ -x "$RF" ] || die "--rf: $RF is not an executable file"
    cp "$RF" "$CONTENTS/MacOS/rf"
    echo "bundled rf from $RF"
fi

# Icon: the committed .icns generated from packaging/brand/ramflux-mark.svg
# by scripts/generate-icons.sh. A missing file only drops the icon.
ICON_FILE=""
if [ -f packaging/macos/AppIcon.icns ]; then
    cp packaging/macos/AppIcon.icns "$CONTENTS/Resources/AppIcon.icns"
    ICON_FILE="<key>CFBundleIconFile</key><string>AppIcon</string>"
else
    echo "package-macos: warning: packaging/macos/AppIcon.icns is missing; the bundle has the default icon" >&2
fi

for notice in LICENSE LICENSING.md NOTICE; do
    [ -f "$notice" ] && cp "$notice" "$CONTENTS/Resources/licenses/$notice"
done

cat >"$CONTENTS/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>$APP_NAME</string>
  <key>CFBundleDisplayName</key><string>$APP_NAME</string>
  <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
  <key>CFBundleExecutable</key><string>$EXECUTABLE</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  $ICON_FILE
  <key>LSMinimumSystemVersion</key><string>$MIN_MACOS</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.social-networking</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSMicrophoneUsageDescription</key><string>Ramflux Desktop uses the microphone for voice calls. Audio is end-to-end encrypted.</string>
  <key>NSLocalNetworkUsageDescription</key><string>Ramflux Desktop connects to Ramflux nodes on your local network, and so does the local daemon it starts.</string>
  <key>NSHumanReadableCopyright</key><string>Copyright (c) 2026 Span Brain. AGPL-3.0-or-later.</string>
  <key>RamfluxBuildRevision</key><string>$REV</string>
</dict>
</plist>
PLIST
plutil -lint "$CONTENTS/Info.plist" >/dev/null || die "Info.plist does not lint"

# Nothing but the expected files may be in the bundle.
unexpected=$(cd "$APP" && find . -type f | sed 's|^\./||' | grep -v -E \
    '^Contents/(Info\.plist|MacOS/(ramflux-desktop|rf)|Resources/AppIcon\.icns|Resources/licenses/[A-Za-z.-]+|_CodeSignature/CodeResources)$' || true)
[ -z "$unexpected" ] || die "unexpected files in the bundle: $unexpected"

if [ "$SIGN" -eq 1 ]; then
    # Inside out: the nested rf first, then the bundle, with one identity
    # (ad hoc unless one is named), so the daemon the app starts is signed
    # like the app that is responsible for it.
    SIGNER="${IDENTITY:--}"
    if [ -f "$CONTENTS/MacOS/rf" ]; then
        codesign --force --sign "$SIGNER" "$CONTENTS/MacOS/rf"
    fi
    codesign --force --sign "$SIGNER" "$APP"
    codesign --verify --strict "$APP" || die "signature does not verify"
    if [ -z "$IDENTITY" ]; then
        echo "package-macos: note: ad hoc signature; macOS may drop the first local network connection after each new build (see docs/local-network.md)" >&2
    fi
fi

echo "built: $APP"
echo "version: $VERSION ($REV), bundle id: $BUNDLE_ID"
echo "run: open \"$APP\""
