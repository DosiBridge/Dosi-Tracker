#!/usr/bin/env bash
#
# Assemble the Linux release artifacts (design 15.1/15.2) from the release binaries:
#
#   cargo build --locked --release --bins
#   bash packaging/build-dist.sh            # usually through: bash ci/check.sh package
#
# Writes into ${DIST_DIR:-<target>/dist} (the directory is recreated):
#   dosi-tracker-X.Y.Z-x86_64-linux-gnu.tar.gz   rootless tarball (install.sh, uninstall.sh, bin/, packaging/)
#   dosi-tracker_X.Y.Z-1_amd64.deb               cargo-deb, from [package.metadata.deb]
#   dosi-tracker-X.Y.Z-1.x86_64.rpm              cargo-generate-rpm, from [package.metadata.generate-rpm]
#   dosi-tracker-linux-x86_64.tar.gz, dosi-tracker_amd64.deb, dosi-tracker.x86_64.rpm
#                                                the same files under version-less names, so
#                                                .../releases/latest/download/<name> works
#   SHA256SUMS                                   sha256sum of all six files
#
# Before packaging it renders, into <target>/release/dist-assets (where the Cargo.toml
# package metadata expects them): the user unit and the desktop entry with
# @BINDIR@ = /usr/bin, and the bash/zsh/fish completions from `dosi-tracker completions`.
# The tarball keeps the templates; install.sh renders them for its prefix.
#
# Environment: CARGO_TARGET_DIR (default <crate>/target), DIST_DIR, SOURCE_DATE_EPOCH
# (tarball timestamps; default: Cargo.toml's mtime), SKIP_PACKAGES=1 (tarball only).

set -euo pipefail

CRATE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
cd "${CRATE_DIR}"

APP_ID="io.github.dosibridge.DosiTracker"
TARGET_DIR="${CARGO_TARGET_DIR:-${CRATE_DIR}/target}"
RELEASE_DIR="${TARGET_DIR}/release"
DIST_DIR="${DIST_DIR:-${TARGET_DIR}/dist}"
ASSETS_DIR="${RELEASE_DIR}/dist-assets"
CLI="${RELEASE_DIR}/dosi-tracker"
GUI="${RELEASE_DIR}/dosi-tracker-gui"

log() {
  printf '==> %s\n' "$*"
}

die() {
  printf 'build-dist: %s\n' "$*" >&2
  exit 1
}

[ -x "${CLI}" ] || die "missing ${CLI}; run: cargo build --locked --release --bins"
[ -x "${GUI}" ] || die "missing ${GUI}; run: cargo build --locked --release --bins"

CRATE_VERSION="$(sed -n 's/^version[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' Cargo.toml | head -n1)"
VERSION="$("${CLI}" --version | awk '{print $2}')"
[ -n "${VERSION}" ] || die "dosi-tracker --version printed no version"
[ "${VERSION}" = "${CRATE_VERSION}" ] || die "the release binary is ${VERSION}, Cargo.toml says ${CRATE_VERSION}; rebuild it"
[ "$("${GUI}" --version)" = "dosi-tracker-gui ${VERSION}" ] || die "dosi-tracker-gui --version does not print ${VERSION}"
SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-$(stat -c %Y Cargo.toml)}"
export SOURCE_DATE_EPOCH

# 1. Rendered assets for the packages (/usr/bin)
log "rendering package assets into ${ASSETS_DIR}"
rm -rf "${ASSETS_DIR}"
mkdir -p "${ASSETS_DIR}/usr" "${ASSETS_DIR}/completions"
sed 's|@BINDIR@|/usr/bin|g' packaging/systemd/dosi-tracker.service.in > "${ASSETS_DIR}/usr/dosi-tracker.service"
sed 's|@BINDIR@|/usr/bin|g' "packaging/desktop/${APP_ID}.desktop" > "${ASSETS_DIR}/usr/${APP_ID}.desktop"
if grep -q '@BINDIR@' "${ASSETS_DIR}/usr/dosi-tracker.service" "${ASSETS_DIR}/usr/${APP_ID}.desktop"; then
  die "unrendered @BINDIR@ left in the package assets"
fi
"${CLI}" completions bash > "${ASSETS_DIR}/completions/dosi-tracker.bash"
"${CLI}" completions zsh > "${ASSETS_DIR}/completions/_dosi-tracker"
"${CLI}" completions fish > "${ASSETS_DIR}/completions/dosi-tracker.fish"
for f in "${ASSETS_DIR}"/completions/*; do
  [ -s "${f}" ] || die "empty completion file ${f}"
done
if command -v desktop-file-validate >/dev/null 2>&1; then
  desktop-file-validate "${ASSETS_DIR}/usr/${APP_ID}.desktop"
fi

rm -rf "${DIST_DIR}"
mkdir -p "${DIST_DIR}"

# 2. Tarball: what install.sh needs in its tarball mode (bin/ next to it)
TARBALL="dosi-tracker-${VERSION}-x86_64-linux-gnu.tar.gz"
log "assembling ${TARBALL}"
STAGE="$(mktemp -d)"
trap 'rm -rf "${STAGE}"' EXIT
TREE="${STAGE}/dosi-tracker-${VERSION}"
mkdir -p "${TREE}/bin" "${TREE}/packaging/systemd" "${TREE}/packaging/desktop"
install -m 0755 scripts/install.sh "${TREE}/install.sh"
install -m 0755 scripts/uninstall.sh "${TREE}/uninstall.sh"
install -m 0755 "${CLI}" "${TREE}/bin/dosi-tracker"
install -m 0755 "${GUI}" "${TREE}/bin/dosi-tracker-gui"
install -m 0644 packaging/systemd/dosi-tracker.service.in "${TREE}/packaging/systemd/"
install -m 0644 "packaging/desktop/${APP_ID}.desktop" "${TREE}/packaging/desktop/"
cp -r packaging/icons "${TREE}/packaging/icons"
rm -f "${TREE}/packaging/icons/render-icons.py"
find "${TREE}/packaging/icons" -type d -exec chmod 0755 {} +
find "${TREE}/packaging/icons" -type f -exec chmod 0644 {} +
install -m 0644 README.md config.example.toml "${TREE}/"
printf '%s\n' "${VERSION}" > "${TREE}/VERSION"
chmod 0644 "${TREE}/VERSION"
tar --sort=name --owner=0 --group=0 --numeric-owner --mtime="@${SOURCE_DATE_EPOCH}" \
  -C "${STAGE}" -cf - "dosi-tracker-${VERSION}" | gzip -n -9 > "${DIST_DIR}/${TARBALL}"

# 3. Packages (no maintainer scripts; nothing enabled)
DEB="dosi-tracker_${VERSION}-1_amd64.deb"
RPM="dosi-tracker-${VERSION}-1.x86_64.rpm"
if [ "${SKIP_PACKAGES:-0}" != "1" ]; then
  command -v cargo-deb >/dev/null 2>&1 || die "cargo-deb is not on PATH (bash ci/check.sh package installs the pinned version)"
  command -v cargo-generate-rpm >/dev/null 2>&1 || die "cargo-generate-rpm is not on PATH (bash ci/check.sh package installs the pinned version)"
  log "building ${DEB}"
  cargo deb --no-build --no-strip --output "${DIST_DIR}/${DEB}"
  log "building ${RPM}"
  cargo generate-rpm --target-dir "${TARGET_DIR}" --output "${DIST_DIR}/${RPM}"
  [ -f "${DIST_DIR}/${DEB}" ] || die "cargo-deb did not write ${DEB}"
  [ -f "${DIST_DIR}/${RPM}" ] || die "cargo-generate-rpm did not write ${RPM}"
fi

# 4. Version-less names (contract: /releases/latest/download/<name>) and checksums
cp "${DIST_DIR}/${TARBALL}" "${DIST_DIR}/dosi-tracker-linux-x86_64.tar.gz"
FILES=("${TARBALL}" dosi-tracker-linux-x86_64.tar.gz)
if [ "${SKIP_PACKAGES:-0}" != "1" ]; then
  cp "${DIST_DIR}/${DEB}" "${DIST_DIR}/dosi-tracker_amd64.deb"
  cp "${DIST_DIR}/${RPM}" "${DIST_DIR}/dosi-tracker.x86_64.rpm"
  FILES+=("${DEB}" dosi-tracker_amd64.deb "${RPM}" dosi-tracker.x86_64.rpm)
fi
(cd "${DIST_DIR}" && sha256sum "${FILES[@]}" > SHA256SUMS)
log "artifacts in ${DIST_DIR}:"
(cd "${DIST_DIR}" && ls -l && cat SHA256SUMS)
