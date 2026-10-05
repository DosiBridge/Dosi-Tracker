#!/usr/bin/env bash
#
# Builds "Dosi Tracker.app" from the Swift package and packs it for
# distribution (the macOS counterpart of clients/windows/installer):
#
#   1. swift build -c release — a universal binary (arm64 + x86_64) by default;
#   2. the .app bundle: the binary, Resources/Info.plist (with the version and,
#      when given, the server addresses baked in) and an icon drawn by
#      scripts/make-icon.swift;
#   3. codesign with the hardened runtime and Resources/DosiTracker.entitlements
#      (the camera; no App Sandbox, which would forbid reading other apps'
#      windows and input): ad-hoc by default, Developer ID when
#      CODESIGN_IDENTITY is set;
#   4. notarization and stapling when NOTARY_PROFILE is set;
#   5. <out>/DosiTracker-<version>-macos.zip, and a .dmg with MAKE_DMG=1.
#
# Usage (on a Mac with Xcode 15 or later):
#
#     bash scripts/build-app.sh
#
# Environment:
#
#   VERSION              CFBundleShortVersionString (default: Resources/Info.plist)
#   BUILD_NUMBER         CFBundleVersion (default: 1)
#   UNIVERSAL            1 (default): arm64 + x86_64; 0: this Mac's architecture only
#   DOSI_API_BASE_URL    bake the server (API) address into Info.plist, e.g.
#                        https://tracker-api.example.com — the sign-in form starts with it
#   DOSI_DASHBOARD_URL   bake the web dashboard address ("Open web dashboard")
#   CODESIGN_IDENTITY    e.g. "Developer ID Application: Example Ltd (TEAMID1234)";
#                        default "-" (ad-hoc)
#   NOTARY_PROFILE       a notarytool keychain profile: notarize and staple
#   MAKE_DMG             1: also build a disk image with an Applications link
#   DIST_DIR             output folder (default: .build/dist)
#
# Signing and notarization (so a downloaded app opens without Gatekeeper's
# warning, and macOS keeps the user's privacy grants across updates):
#
#   1. Install a "Developer ID Application" certificate in the login keychain.
#   2. Once, store notary credentials (an app-specific password of the Apple ID):
#        xcrun notarytool store-credentials dosi-notary \
#          --apple-id you@example.com --team-id TEAMID1234 --password abcd-efgh-ijkl-mnop
#   3. Build:
#        CODESIGN_IDENTITY="Developer ID Application: Example Ltd (TEAMID1234)" \
#        NOTARY_PROFILE=dosi-notary MAKE_DMG=1 bash scripts/build-app.sh
#      The app is submitted to Apple's notary service, the ticket is stapled to
#      it, and the zip (and dmg) are made from the stapled app.
#
# An ad-hoc signed build runs on the Mac that built it; elsewhere Gatekeeper
# asks the user to open it from System Settings → Privacy & Security, and every
# new build counts as a new app for the privacy permissions.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PACKAGE_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PACKAGE_DIR"

APP_NAME="Dosi Tracker"
EXECUTABLE="DosiTracker"
INFO_PLIST="Resources/Info.plist"
ENTITLEMENTS="Resources/DosiTracker.entitlements"
CAMERA_ENTITLEMENT="com.apple.security.device.camera"

DIST_DIR="${DIST_DIR:-.build/dist}"
VERSION="${VERSION:-$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$INFO_PLIST")}"
BUILD_NUMBER="${BUILD_NUMBER:-1}"
UNIVERSAL="${UNIVERSAL:-1}"
IDENTITY="${CODESIGN_IDENTITY:--}"
NOTARY_PROFILE="${NOTARY_PROFILE:-}"
MAKE_DMG="${MAKE_DMG:-0}"

step() { printf '\n==> %s\n' "$*"; }
fail() { printf 'build-app: %s\n' "$*" >&2; exit 1; }

[ "$(uname -s)" = "Darwin" ] || fail "the app is built on macOS (Xcode 15 or later)"
[[ "$VERSION" =~ ^[0-9]+(\.[0-9]+){1,2}$ ]] || fail "VERSION must look like 1.2 or 1.2.3 (got '$VERSION')"
[[ "$BUILD_NUMBER" =~ ^[0-9]+(\.[0-9]+)*$ ]] || fail "BUILD_NUMBER must be numeric (got '$BUILD_NUMBER')"
if [ -n "$NOTARY_PROFILE" ] && [ "$IDENTITY" = "-" ]; then
  fail "notarization needs a Developer ID signature: set CODESIGN_IDENTITY"
fi
url_pattern='^https?://[^[:space:]"<>]+$'
for address in "${DOSI_API_BASE_URL:-}" "${DOSI_DASHBOARD_URL:-}"; do
  if [ -n "$address" ] && ! [[ "$address" =~ $url_pattern ]]; then
    fail "server addresses must be http(s) URLs (got '$address')"
  fi
done

WORK="$(mktemp -d "${TMPDIR:-/tmp}/dosi-app.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
mkdir -p "$DIST_DIR"

# 1. Compile ------------------------------------------------------------------
if [ "$UNIVERSAL" = "1" ]; then
  step "Compiling $EXECUTABLE $VERSION for arm64 and x86_64"
  binaries=()
  for arch in arm64 x86_64; do
    swift build -c release --arch "$arch"
    binaries+=("$(swift build -c release --arch "$arch" --show-bin-path)/$EXECUTABLE")
  done
  lipo -create -output "$WORK/$EXECUTABLE" "${binaries[@]}"
else
  step "Compiling $EXECUTABLE $VERSION for $(uname -m)"
  swift build -c release
  cp "$(swift build -c release --show-bin-path)/$EXECUTABLE" "$WORK/$EXECUTABLE"
fi
lipo -info "$WORK/$EXECUTABLE"

# 2. Bundle -------------------------------------------------------------------
APP="$DIST_DIR/$APP_NAME.app"
PLIST="$APP/Contents/Info.plist"
step "Assembling $APP"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$WORK/$EXECUTABLE" "$APP/Contents/MacOS/$EXECUTABLE"
chmod 755 "$APP/Contents/MacOS/$EXECUTABLE"
cp "$INFO_PLIST" "$PLIST"
plutil -replace CFBundleShortVersionString -string "$VERSION" "$PLIST"
plutil -replace CFBundleVersion -string "$BUILD_NUMBER" "$PLIST"
if [ -n "${DOSI_API_BASE_URL:-}" ]; then
  plutil -replace DosiApiBaseURL -string "$DOSI_API_BASE_URL" "$PLIST"
  echo "server address: $DOSI_API_BASE_URL"
fi
if [ -n "${DOSI_DASHBOARD_URL:-}" ]; then
  plutil -replace DosiDashboardURL -string "$DOSI_DASHBOARD_URL" "$PLIST"
  echo "web dashboard: $DOSI_DASHBOARD_URL"
fi
printf 'APPL????' > "$APP/Contents/PkgInfo"

if swift "$SCRIPT_DIR/make-icon.swift" "$WORK/AppIcon.iconset" \
  && iconutil --convert icns --output "$APP/Contents/Resources/AppIcon.icns" "$WORK/AppIcon.iconset"; then
  echo "icon: AppIcon.icns"
else
  echo "warning: the app icon could not be made; the bundle shows the generic icon" >&2
  plutil -remove CFBundleIconFile "$PLIST" || true
fi
plutil -lint "$PLIST"

# 3. Sign ---------------------------------------------------------------------
if [ "$IDENTITY" = "-" ]; then
  step "Signing (ad-hoc, hardened runtime)"
  codesign --force --options runtime --entitlements "$ENTITLEMENTS" --sign - "$APP"
else
  step "Signing as $IDENTITY (hardened runtime)"
  codesign --force --options runtime --timestamp --entitlements "$ENTITLEMENTS" --sign "$IDENTITY" "$APP"
fi
codesign --verify --strict --verbose=2 "$APP"
# Read whole, then checked (a `grep -q` on the pipe can end codesign early,
# which pipefail would report as a failure).
signature="$(codesign --display --verbose=2 "$APP" 2>&1)"
[[ "$signature" == *runtime* ]] || fail "the signature has no hardened runtime"
entitlements="$(codesign --display --entitlements - "$APP" 2>&1)"
[[ "$entitlements" == *"$CAMERA_ENTITLEMENT"* ]] || fail "the signature lacks the camera entitlement"

# 4. Notarize -----------------------------------------------------------------
if [ -n "$NOTARY_PROFILE" ]; then
  step "Notarizing (profile $NOTARY_PROFILE)"
  ditto -c -k --sequesterRsrc --keepParent "$APP" "$WORK/notarize.zip"
  xcrun notarytool submit "$WORK/notarize.zip" --keychain-profile "$NOTARY_PROFILE" --wait
  xcrun stapler staple "$APP"
  xcrun stapler validate "$APP"
  spctl --assess --type execute --verbose=2 "$APP"
fi

# 5. Pack ---------------------------------------------------------------------
ZIP="$DIST_DIR/DosiTracker-$VERSION-macos.zip"
step "Packing $ZIP"
rm -f "$ZIP"
ditto -c -k --sequesterRsrc --keepParent "$APP" "$ZIP"

if [ "$MAKE_DMG" = "1" ]; then
  DMG="$DIST_DIR/DosiTracker-$VERSION-macos.dmg"
  step "Building $DMG"
  STAGE="$WORK/dmg"
  mkdir -p "$STAGE"
  ditto "$APP" "$STAGE/$APP_NAME.app"
  ln -s /Applications "$STAGE/Applications"
  rm -f "$DMG"
  hdiutil create -volname "$APP_NAME" -srcfolder "$STAGE" -ov -format UDZO "$DMG"
  if [ "$IDENTITY" != "-" ]; then
    codesign --force --timestamp --sign "$IDENTITY" "$DMG"
  fi
  if [ -n "$NOTARY_PROFILE" ]; then
    xcrun notarytool submit "$DMG" --keychain-profile "$NOTARY_PROFILE" --wait
    xcrun stapler staple "$DMG"
  fi
fi

step "Done"
for file in "$ZIP" ${DMG:+"$DMG"}; do
  printf '%s  %s\n' "$(shasum -a 256 "$file" | cut -d ' ' -f 1)" "$file"
done
