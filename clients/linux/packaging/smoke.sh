#!/usr/bin/env bash
#
# Distro smoke test of the release artifacts (design 15.4). Runs as root inside a
# throw-away stock distro container, with the dist directory mounted read-only:
#
#   docker run --rm -v <dist>:/dist:ro -v <clients/linux/packaging>:/pkg:ro \
#     ubuntu:24.04 bash /pkg/smoke.sh /dist
#
# Ubuntu and Debian install the .deb with apt, Fedora the .rpm with dnf (weak
# dependencies off, so the hard dependencies alone must be enough), Arch installs
# the tarball as a non-root user. Each checks: SHA256SUMS, `--version` of both
# programs, `doctor --json` (one JSON document, nothing created), `status` without
# an agent (exit 3, nothing created), `systemd-analyze verify --user` of the unit,
# `desktop-file-validate` of the app entry, the unit installed but not enabled,
# nothing in /etc/xdg/autostart (or ~/.config/autostart), and a clean removal.

set -euo pipefail

DIST="${1:?usage: smoke.sh <dist directory>}"
APP_ID="io.github.dosibridge.DosiTracker"
SMOKE_USER="dosismoke"
CLEAN_PATH="/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"

log() {
  printf '\n==> [%s] %s\n' "${DISTRO:-?}" "$*"
}

die() {
  printf 'SMOKE FAILED [%s]: %s\n' "${DISTRO:-?}" "$*" >&2
  exit 1
}

[ "$(id -u)" -eq 0 ] || die "run this as root inside a throw-away container (it installs packages)"
# shellcheck disable=SC1091
. /etc/os-release
DISTRO="${ID}-${VERSION_ID:-rolling}"

log "checksums"
(cd "${DIST}" && sha256sum -c SHA256SUMS) || die "SHA256SUMS does not match"
TARBALL="$(cd "${DIST}" && ls dosi-tracker-*-x86_64-linux-gnu.tar.gz)"
VERSION="${TARBALL#dosi-tracker-}"
VERSION="${VERSION%-x86_64-linux-gnu.tar.gz}"
[ -n "${VERSION}" ] || die "no versioned tarball in ${DIST}"

# Runs "$@" as the unprivileged smoke user with a clean environment.
as_user() {
  local home
  home="$(getent passwd "${SMOKE_USER}" | cut -d: -f6)"
  runuser -u "${SMOKE_USER}" -- env -i HOME="${home}" USER="${SMOKE_USER}" LANG=C.UTF-8 \
    PATH="${home}/.local/bin:${CLEAN_PATH}" XDG_RUNTIME_DIR="${home}/run" "$@"
}

add_smoke_user() {
  useradd --create-home --shell /bin/bash "${SMOKE_USER}"
  local home
  home="$(getent passwd "${SMOKE_USER}" | cut -d: -f6)"
  install -d -m 0700 -o "${SMOKE_USER}" -g "$(id -gn "${SMOKE_USER}")" "${home}/run"
}

# Checks shared by every install method. $1 = bin dir, $2 = unit, $3 = desktop entry.
check_installed() {
  local bindir="$1" unit="$2" entry="$3"
  local home
  home="$(getent passwd "${SMOKE_USER}" | cut -d: -f6)"

  log "--version"
  [ "$(as_user "${bindir}/dosi-tracker" --version)" = "dosi-tracker ${VERSION}" ] \
    || die "dosi-tracker --version is not 'dosi-tracker ${VERSION}'"
  [ "$(as_user "${bindir}/dosi-tracker-gui" --version)" = "dosi-tracker-gui ${VERSION}" ] \
    || die "dosi-tracker-gui --version is not 'dosi-tracker-gui ${VERSION}'"

  log "doctor --json and status create nothing"
  local stamp="${home}/run/.stamp"
  as_user touch "${stamp}"
  sleep 1
  as_user "${bindir}/dosi-tracker" doctor --json > /tmp/doctor.json || die "doctor --json failed"
  python3 -m json.tool /tmp/doctor.json >/dev/null || die "doctor --json is not one JSON document"
  grep -q '"dosi-tracker-linux"' /tmp/doctor.json || die "doctor --json does not name the client"
  local code=0
  as_user "${bindir}/dosi-tracker" status --no-start >/dev/null 2>&1 || code=$?
  [ "${code}" -eq 3 ] || die "status without an agent exited ${code}, not 3"
  local created
  created="$(find "${home}" -newer "${stamp}" -not -path "${home}/run" -not -path "${home}")"
  [ -z "${created}" ] || die "doctor/status created files: ${created}"

  log "systemd-analyze verify --user ${unit}"
  [ -f "${unit}" ] || die "the user unit ${unit} is missing"
  grep -qx "ExecStart=${bindir}/dosi-tracker daemon --systemd" "${unit}" || die "ExecStart does not point at ${bindir}"
  if grep -q '^\[Install\]' "${unit}"; then
    die "the unit has an [Install] section"
  fi
  as_user systemd-analyze verify --user "${unit}" || die "systemd-analyze verify failed"

  log "desktop-file-validate ${entry}"
  desktop-file-validate "${entry}" || die "the desktop entry does not validate"
  grep -qx "Exec=${bindir}/dosi-tracker-gui" "${entry}" || die "the desktop entry does not run ${bindir}/dosi-tracker-gui"
  for size in 16x16 32x32 48x48 128x128 256x256 512x512 scalable; do
    ls "$(dirname "$(dirname "${entry}")")/icons/hicolor/${size}/apps/${APP_ID}".* >/dev/null 2>&1 \
      || die "the ${size} icon is missing"
  done

  log "nothing enabled, nothing started at login"
  if [ -n "$(find /etc/xdg/autostart "${home}/.config/autostart" -name '*dosi*' 2>/dev/null)" ]; then
    die "an autostart entry was installed"
  fi
  if [ -n "$(find /etc/systemd /usr/lib/systemd "${home}/.config/systemd" "${home}/.local/share/systemd" \
    -path '*.wants/*' -name 'dosi-tracker*' 2>/dev/null)" ]; then
    die "the unit is enabled"
  fi
}

# Every file of the package must be gone after removal ($1 = file list).
check_removed() {
  local list="$1" path
  while IFS= read -r path; do
    [ -n "${path}" ] || continue
    if [ -e "${path}" ] && [ ! -d "${path}" ]; then
      die "${path} is still there after removal"
    fi
  done < "${list}"
}

case "${ID}" in
  ubuntu|debian)
    log "apt: install the .deb (no recommends)"
    export DEBIAN_FRONTEND=noninteractive
    apt-get update -qq
    apt-get install -y -qq --no-install-recommends systemd desktop-file-utils python3 passwd util-linux >/dev/null
    apt-get install -y --no-install-recommends "${DIST}/dosi-tracker_amd64.deb"
    add_smoke_user
    dpkg -L dosi-tracker | sort > /tmp/files
    if grep -Eq '^/etc(/|$)' /tmp/files; then
      die "the package installs files under /etc"
    fi
    check_installed /usr/bin /usr/lib/systemd/user/dosi-tracker.service "/usr/share/applications/${APP_ID}.desktop"
    log "apt: remove and purge"
    apt-get remove -y --purge dosi-tracker
    if dpkg -s dosi-tracker >/dev/null 2>&1; then
      die "dosi-tracker is still installed"
    fi
    check_removed /tmp/files
    ;;
  fedora)
    log "dnf: install the .rpm (no weak dependencies)"
    dnf install -y -q --setopt=install_weak_deps=False systemd desktop-file-utils python3 shadow-utils util-linux >/dev/null
    dnf install -y --setopt=install_weak_deps=False "${DIST}/dosi-tracker.x86_64.rpm"
    add_smoke_user
    rpm -ql dosi-tracker | sort > /tmp/files
    if grep -Eq '^/etc(/|$)' /tmp/files; then
      die "the package installs files under /etc"
    fi
    if [ -n "$(rpm -q --scripts dosi-tracker)" ]; then
      die "the package has scriptlets"
    fi
    check_installed /usr/bin /usr/lib/systemd/user/dosi-tracker.service "/usr/share/applications/${APP_ID}.desktop"
    log "dnf: remove"
    dnf remove -y dosi-tracker
    if rpm -q dosi-tracker >/dev/null 2>&1; then
      die "dosi-tracker is still installed"
    fi
    check_removed /tmp/files
    ;;
  arch)
    log "pacman: tools for the tarball install (python3 validates the manifest)"
    pacman -Syu --noconfirm --needed python desktop-file-utils systemd util-linux >/dev/null
    add_smoke_user
    home="$(getent passwd "${SMOKE_USER}" | cut -d: -f6)"
    cp "${DIST}/dosi-tracker-linux-x86_64.tar.gz" "${home}/"
    chown "${SMOKE_USER}" "${home}/dosi-tracker-linux-x86_64.tar.gz"
    log "tarball: install as ${SMOKE_USER} into ~/.local"
    as_user bash -c "cd ~ && tar -xzf dosi-tracker-linux-x86_64.tar.gz && ./dosi-tracker-${VERSION}/install.sh"
    check_installed "${home}/.local/bin" "${home}/.local/share/systemd/user/dosi-tracker.service" \
      "${home}/.local/share/applications/${APP_ID}.desktop"
    if [ -n "$(find "${home}/.local" "${home}/.config" -user root 2>/dev/null)" ]; then
      die "the tarball install created root-owned files"
    fi
    log "tarball: uninstall"
    as_user bash "${home}/.local/lib/dosi-tracker/install/uninstall.sh" --yes
    left="$(find "${home}/.local" -type f)"
    [ -z "${left}" ] || die "files left after uninstall: ${left}"
    ;;
  *)
    die "no smoke recipe for ${ID}"
    ;;
esac

log "passed"
