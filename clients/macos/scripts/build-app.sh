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
#      windows and input): Developer ID when a signing identity is available,
#      ad-hoc otherwise;
#   4. notarization and stapling when notary credentials are available;
#   5. <out>/DosiTracker-<version>-macos.zip and the version-less release asset
#      DosiTracker-macos.zip (plus .dmg files with MAKE_DMG=1), and SHA256SUMS.
#
# Usage (on a Mac with Xcode 15 or later):
#
#     bash scripts/build-app.sh            build
#     bash scripts/build-app.sh --plan     print what would be built, and exit
#
# No server address is baked in unless DOSI_API_BASE_URL is set: the built app
# then asks the user for it at the first sign-in (it never signs in to the
# development default, https://localhost:6006).
#
# Environment (every signing and notary variable is optional; the steps run
# only when their credentials are present, and a half-configured set fails):
#
#   VERSION              CFBundleShortVersionString, e.g. 1.4.0 or v1.4.0.
#                        Default: the tag being built (GITHUB_REF refs/tags/v1.4.0),
#                        else Resources/Info.plist. A pre-release suffix
#                        (v1.4.0-rc.1) is dropped: macOS versions are numeric.
#   BUILD_NUMBER         CFBundleVersion (default: GITHUB_RUN_NUMBER, else 1)
#   UNIVERSAL            1 (default): arm64 + x86_64; 0: this Mac's architecture only
#   DOSI_API_BASE_URL    bake the server (API) address into Info.plist, e.g.
#                        https://tracker-api.example.com — the sign-in form starts with it
#   DOSI_DASHBOARD_URL   bake the web dashboard address ("Open web dashboard")
#   MAKE_DMG             1: also build a disk image with an Applications link
#   DIST_DIR             output folder (default: .build/dist)
#   REQUIRE_NOTARIZATION 1: fail unless the app is Developer ID signed and
#                        notarized (set it in release jobs)
#
#   Signing — one of:
#   DEVELOPER_ID_P12_BASE64, DEVELOPER_ID_P12_PASSWORD
#                        a "Developer ID Application" certificate and its key
#                        (.p12, base64): imported into a temporary keychain that
#                        is deleted afterwards (CI secrets)
#   CODESIGN_IDENTITY    e.g. "Developer ID Application: Example Ltd (TEAMID1234)":
#                        an identity in your keychain; with the .p12, picks one of it
#
#   Notarization — one of:
#   NOTARY_PROFILE       a notarytool keychain profile (xcrun notarytool store-credentials)
#   NOTARY_API_KEY_BASE64 (or NOTARY_API_KEY_PATH), NOTARY_API_KEY_ID, NOTARY_API_ISSUER_ID
#                        an App Store Connect API key (.p8)
#   NOTARY_APPLE_ID, NOTARY_TEAM_ID, NOTARY_PASSWORD
#                        an Apple ID with an app-specific password
#
# An ad-hoc signed build is for development only: it runs on the Mac that built
# it; elsewhere Gatekeeper blocks it until it is allowed in System Settings →
# Privacy & Security, and every new build counts as a new app for the privacy
# permissions and the Keychain. Ship Developer ID signed, notarized builds.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PACKAGE_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PACKAGE_DIR"

APP_NAME="Dosi Tracker"
EXECUTABLE="DosiTracker"
INFO_PLIST="Resources/Info.plist"
ENTITLEMENTS="Resources/DosiTracker.entitlements"
CAMERA_ENTITLEMENT="com.apple.security.device.camera"
# The release asset names other tools link to (/releases/latest/download/<name>).
RELEASE_ZIP="DosiTracker-macos.zip"
RELEASE_DMG="DosiTracker-macos.dmg"

PLAN_ONLY=0
case "${1:-}" in
  "") ;;
  --plan) PLAN_ONLY=1 ;;
  *) printf 'usage: build-app.sh [--plan]\n' >&2; exit 2 ;;
esac

step() { printf '\n==> %s\n' "$*"; }
fail() { printf 'build-app: %s\n' "$*" >&2; exit 1; }
warn() {
  printf 'build-app: warning: %s\n' "$*" >&2
  if [ "${GITHUB_ACTIONS:-}" = "true" ]; then
    printf '::warning::%s\n' "$*"
  fi
}

# Version ----------------------------------------------------------------------
plist_version() {
  if [ -x /usr/libexec/PlistBuddy ]; then
    /usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$INFO_PLIST"
  else
    # --plan off a Mac: the key's <string> on the line after it.
    sed -n '/<key>CFBundleShortVersionString<\/key>/{n;s/.*<string>\(.*\)<\/string>.*/\1/p;}' "$INFO_PLIST"
  fi
}

raw_version="${VERSION:-}"
version_source="VERSION"
if [ -z "$raw_version" ]; then
  ref="${GITHUB_REF:-}"
  if [ "$ref" != "${ref#refs/tags/}" ]; then
    raw_version="${ref#refs/tags/}"
    version_source="tag $raw_version"
  else
    raw_version="$(plist_version)"
    version_source="$INFO_PLIST"
  fi
fi
VERSION="${raw_version#v}"
if [[ "$VERSION" =~ ^([0-9]+(\.[0-9]+){1,2})[-+].+$ ]]; then
  warn "macOS versions are numeric: '$raw_version' is built as ${BASH_REMATCH[1]}"
  VERSION="${BASH_REMATCH[1]}"
fi
[[ "$VERSION" =~ ^[0-9]+(\.[0-9]+){1,2}$ ]] || fail "the version must look like 1.2 or 1.2.3 (got '$raw_version' from $version_source)"
BUILD_NUMBER="${BUILD_NUMBER:-${GITHUB_RUN_NUMBER:-1}}"
[[ "$BUILD_NUMBER" =~ ^[0-9]+(\.[0-9]+)*$ ]] || fail "BUILD_NUMBER must be numeric (got '$BUILD_NUMBER')"
UNIVERSAL="${UNIVERSAL:-1}"
MAKE_DMG="${MAKE_DMG:-0}"
DIST_DIR="${DIST_DIR:-.build/dist}"
REQUIRE_NOTARIZATION="${REQUIRE_NOTARIZATION:-0}"

# Signing and notarization: what the environment provides ------------------------
P12_BASE64="${DEVELOPER_ID_P12_BASE64:-}"
P12_PASSWORD="${DEVELOPER_ID_P12_PASSWORD:-}"
IDENTITY="${CODESIGN_IDENTITY:-}"
if [ -n "$P12_BASE64" ] || [ -n "$P12_PASSWORD" ]; then
  [ -n "$P12_BASE64" ] && [ -n "$P12_PASSWORD" ] \
    || fail "signing needs both DEVELOPER_ID_P12_BASE64 and DEVELOPER_ID_P12_PASSWORD"
  SIGNING="developer-id (certificate from the environment, temporary keychain)"
elif [ -n "$IDENTITY" ] && [ "$IDENTITY" != "-" ]; then
  SIGNING="developer-id ($IDENTITY, from your keychain)"
else
  IDENTITY="-"
  SIGNING="ad-hoc (development only: not for distribution)"
fi
signed_for_distribution() { [ "$IDENTITY" != "-" ] || [ -n "$P12_BASE64" ]; }

API_KEY_BASE64="${NOTARY_API_KEY_BASE64:-}"
API_KEY_PATH="${NOTARY_API_KEY_PATH:-}"
NOTARY=""
notary_sets=0
if [ -n "${NOTARY_PROFILE:-}" ]; then
  NOTARY="keychain profile ${NOTARY_PROFILE}"
  notary_sets=$((notary_sets + 1))
fi
if [ -n "$API_KEY_BASE64$API_KEY_PATH${NOTARY_API_KEY_ID:-}${NOTARY_API_ISSUER_ID:-}" ]; then
  { [ -n "$API_KEY_BASE64" ] || [ -n "$API_KEY_PATH" ]; } && [ -n "${NOTARY_API_KEY_ID:-}" ] \
    && [ -n "${NOTARY_API_ISSUER_ID:-}" ] \
    || fail "an App Store Connect key needs NOTARY_API_KEY_BASE64 (or NOTARY_API_KEY_PATH), NOTARY_API_KEY_ID and NOTARY_API_ISSUER_ID"
  NOTARY="App Store Connect API key ${NOTARY_API_KEY_ID}"
  notary_sets=$((notary_sets + 1))
fi
if [ -n "${NOTARY_APPLE_ID:-}${NOTARY_TEAM_ID:-}${NOTARY_PASSWORD:-}" ]; then
  [ -n "${NOTARY_APPLE_ID:-}" ] && [ -n "${NOTARY_TEAM_ID:-}" ] && [ -n "${NOTARY_PASSWORD:-}" ] \
    || fail "an Apple ID needs NOTARY_APPLE_ID, NOTARY_TEAM_ID and NOTARY_PASSWORD"
  NOTARY="Apple ID (team ${NOTARY_TEAM_ID})"
  notary_sets=$((notary_sets + 1))
fi
[ "$notary_sets" -le 1 ] || fail "set one notarization method, not $notary_sets"
if [ -n "$NOTARY" ] && ! signed_for_distribution; then
  fail "notarization needs a Developer ID signature: set DEVELOPER_ID_P12_BASE64 or CODESIGN_IDENTITY"
fi
if [ "$REQUIRE_NOTARIZATION" = "1" ]; then
  signed_for_distribution || fail "REQUIRE_NOTARIZATION=1, but no Developer ID signing identity is configured"
  [ -n "$NOTARY" ] || fail "REQUIRE_NOTARIZATION=1, but no notary credentials are configured"
fi

# Server addresses ----------------------------------------------------------------
url_pattern='^https?://[^[:space:]"<>]+$'
for address in "${DOSI_API_BASE_URL:-}" "${DOSI_DASHBOARD_URL:-}"; do
  if [ -n "$address" ] && ! [[ "$address" =~ $url_pattern ]]; then
    fail "server addresses must be http(s) URLs (got '$address')"
  fi
done
# A build for other Macs never points at a development server.
loopback_pattern='^https?://(localhost|[^/:]*\.localhost|127\.[0-9.]+|\[::1\])([:/]|$)'
if [ -n "${DOSI_API_BASE_URL:-}" ] && signed_for_distribution; then
  shopt -s nocasematch
  if [[ "$DOSI_API_BASE_URL" =~ $loopback_pattern ]]; then
    fail "a Developer ID build must not bake in a local server ($DOSI_API_BASE_URL)"
  fi
  if [[ "$DOSI_API_BASE_URL" != https://* ]]; then
    fail "a Developer ID build must bake in an https:// server (got $DOSI_API_BASE_URL)"
  fi
  shopt -u nocasematch
fi

ZIP="$DIST_DIR/DosiTracker-$VERSION-macos.zip"
DMG="$DIST_DIR/DosiTracker-$VERSION-macos.dmg"

step "Plan"
printf '  version        %s (build %s, from %s)\n' "$VERSION" "$BUILD_NUMBER" "$version_source"
printf '  architectures  %s\n' "$([ "$UNIVERSAL" = "1" ] && echo "arm64 + x86_64" || echo "this Mac's")"
printf '  signing        %s\n' "$SIGNING"
printf '  notarization   %s\n' "${NOTARY:-none}"
printf '  server         %s\n' "${DOSI_API_BASE_URL:-none baked in: the app asks at sign-in}"
printf '  dashboard      %s\n' "${DOSI_DASHBOARD_URL:-none baked in}"
printf '  output         %s, %s%s\n' "$ZIP" "$DIST_DIR/$RELEASE_ZIP" \
  "$([ "$MAKE_DMG" = "1" ] && echo ", $DMG, $DIST_DIR/$RELEASE_DMG")"
if [ "$PLAN_ONLY" = "1" ]; then
  exit 0
fi

[ "$(uname -s)" = "Darwin" ] || fail "the app is built on macOS (Xcode 15 or later)"
if ! signed_for_distribution; then
  warn "this build is ad-hoc signed: for development only (Gatekeeper blocks it on other Macs)"
fi

WORK="$(mktemp -d "${TMPDIR:-/tmp}/dosi-app.XXXXXX")"
KEYCHAIN=""
SAVED_KEYCHAINS=()
cleanup() {
  if [ -n "$KEYCHAIN" ]; then
    if [ "${#SAVED_KEYCHAINS[@]}" -gt 0 ]; then
      security list-keychains -d user -s "${SAVED_KEYCHAINS[@]}" || true
    fi
    security delete-keychain "$KEYCHAIN" || true
  fi
  rm -rf "$WORK"
}
trap cleanup EXIT
mkdir -p "$DIST_DIR"

# A Developer ID certificate from the environment: into a keychain of its own,
# found by codesign, deleted on exit.
KEYCHAIN_ARGS=()
if [ -n "$P12_BASE64" ]; then
  step "Importing the Developer ID certificate into a temporary keychain"
  KEYCHAIN="$WORK/signing.keychain-db"
  keychain_password="$(uuidgen)"
  (umask 077 && printf '%s' "$P12_BASE64" | base64 --decode > "$WORK/developer-id.p12") \
    || fail "DEVELOPER_ID_P12_BASE64 is not valid base64"
  security create-keychain -p "$keychain_password" "$KEYCHAIN"
  security set-keychain-settings -lut 21600 "$KEYCHAIN"
  security unlock-keychain -p "$keychain_password" "$KEYCHAIN"
  security import "$WORK/developer-id.p12" -k "$KEYCHAIN" -f pkcs12 -P "$P12_PASSWORD" \
    -T /usr/bin/codesign -T /usr/bin/security >/dev/null
  rm -f "$WORK/developer-id.p12"
  security set-key-partition-list -S apple-tool:,apple: -s -k "$keychain_password" "$KEYCHAIN" >/dev/null
  while IFS= read -r line; do
    line="${line#"${line%%[![:space:]]*}"}"
    line="${line#\"}"
    line="${line%\"}"
    [ -n "$line" ] && SAVED_KEYCHAINS+=("$line")
  done < <(security list-keychains -d user)
  security list-keychains -d user -s "$KEYCHAIN" ${SAVED_KEYCHAINS[@]+"${SAVED_KEYCHAINS[@]}"}
  identities="$(security find-identity -v -p codesigning "$KEYCHAIN")"
  if [ -z "$IDENTITY" ] || [ "$IDENTITY" = "-" ]; then
    IDENTITY="$(printf '%s\n' "$identities" | sed -n 's/^.*"\(Developer ID Application: .*\)"$/\1/p' | head -n 1)"
    [ -n "$IDENTITY" ] || fail "the certificate holds no \"Developer ID Application\" identity"
  fi
  KEYCHAIN_ARGS=(--keychain "$KEYCHAIN")
  echo "identity: $IDENTITY"
fi

# Notarization credentials, as notarytool arguments.
NOTARY_ARGS=()
if [ -n "${NOTARY_PROFILE:-}" ]; then
  NOTARY_ARGS=(--keychain-profile "$NOTARY_PROFILE")
elif [ -n "${NOTARY_API_KEY_ID:-}" ]; then
  key_file="$API_KEY_PATH"
  if [ -n "$API_KEY_BASE64" ]; then
    key_file="$WORK/AuthKey_${NOTARY_API_KEY_ID}.p8"
    (umask 077 && printf '%s' "$API_KEY_BASE64" | base64 --decode > "$key_file") \
      || fail "NOTARY_API_KEY_BASE64 is not valid base64"
  fi
  [ -f "$key_file" ] || fail "the App Store Connect key file does not exist ($key_file)"
  NOTARY_ARGS=(--key "$key_file" --key-id "$NOTARY_API_KEY_ID" --issuer "$NOTARY_API_ISSUER_ID")
elif [ -n "${NOTARY_APPLE_ID:-}" ]; then
  NOTARY_ARGS=(--apple-id "$NOTARY_APPLE_ID" --team-id "$NOTARY_TEAM_ID" --password "$NOTARY_PASSWORD")
fi
notarize() {
  xcrun notarytool submit "$1" "${NOTARY_ARGS[@]}" --wait --timeout 30m
}

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
  codesign --force --options runtime --timestamp --entitlements "$ENTITLEMENTS" --sign "$IDENTITY" \
    ${KEYCHAIN_ARGS[@]+"${KEYCHAIN_ARGS[@]}"} "$APP"
fi
codesign --verify --strict --verbose=2 "$APP"
# Read whole, then checked (a `grep -q` on the pipe can end codesign early,
# which pipefail would report as a failure).
signature="$(codesign --display --verbose=2 "$APP" 2>&1)"
[[ "$signature" == *runtime* ]] || fail "the signature has no hardened runtime"
entitlements="$(codesign --display --entitlements - "$APP" 2>&1)"
[[ "$entitlements" == *"$CAMERA_ENTITLEMENT"* ]] || fail "the signature lacks the camera entitlement"
if [ "$IDENTITY" != "-" ]; then
  # The designated requirement MDM profiles (PPPC, managed login items) match.
  echo "designated requirement: $(codesign --display --requirements - "$APP" 2>&1 | sed -n 's/^designated => //p')"
fi

# 4. Notarize -----------------------------------------------------------------
if [ "${#NOTARY_ARGS[@]}" -gt 0 ]; then
  step "Notarizing ($NOTARY)"
  ditto -c -k --sequesterRsrc --keepParent "$APP" "$WORK/notarize.zip"
  notarize "$WORK/notarize.zip"
  xcrun stapler staple "$APP"
  xcrun stapler validate "$APP"
  spctl --assess --type execute --verbose=2 "$APP"
fi

# 5. Pack ---------------------------------------------------------------------
step "Packing $ZIP"
rm -f "$ZIP" "$DIST_DIR/$RELEASE_ZIP"
ditto -c -k --sequesterRsrc --keepParent "$APP" "$ZIP"
cp "$ZIP" "$DIST_DIR/$RELEASE_ZIP"
outputs=("$ZIP" "$DIST_DIR/$RELEASE_ZIP")

if [ "$MAKE_DMG" = "1" ]; then
  step "Building $DMG"
  STAGE="$WORK/dmg"
  mkdir -p "$STAGE"
  ditto "$APP" "$STAGE/$APP_NAME.app"
  ln -s /Applications "$STAGE/Applications"
  rm -f "$DMG" "$DIST_DIR/$RELEASE_DMG"
  hdiutil create -volname "$APP_NAME" -srcfolder "$STAGE" -ov -format UDZO "$DMG"
  if [ "$IDENTITY" != "-" ]; then
    codesign --force --timestamp --sign "$IDENTITY" ${KEYCHAIN_ARGS[@]+"${KEYCHAIN_ARGS[@]}"} "$DMG"
  fi
  if [ "${#NOTARY_ARGS[@]}" -gt 0 ]; then
    notarize "$DMG"
    xcrun stapler staple "$DMG"
  fi
  cp "$DMG" "$DIST_DIR/$RELEASE_DMG"
  outputs+=("$DMG" "$DIST_DIR/$RELEASE_DMG")
fi

step "Done"
(
  cd "$DIST_DIR"
  for file in "${outputs[@]}"; do
    shasum -a 256 "$(basename "$file")"
  done
) | tee "$DIST_DIR/SHA256SUMS"
