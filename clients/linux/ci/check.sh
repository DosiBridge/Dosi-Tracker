#!/usr/bin/env bash
#
# Linux agent CI stages. Runs inside the ci/Dockerfile image (locally through
# Docker, in CI through .github/workflows/linux.yml) from clients/linux:
#
#   bash ci/check.sh all
#   bash ci/check.sh fmt clippy test installer release
#   bash ci/check.sh package
#
# Stages:
#   fmt        cargo fmt --check
#   clippy     clippy -D warnings for the default, no-default and all feature sets
#   test       cargo test (default features, then with test-support)
#   harness    the #[ignore]d display/bus tests inside ci/xvfb-session.sh
#   installer  tests/test_installer_lifecycle.sh as a non-root user
#   release    release build, release-mode tests, dynamic-link allowlist,
#              --version, doctor --json, the outbox fail-closed checks, no
#              test hooks in the binary, and argv passwords / status
#              without an agent failing closed
#   package    release build, then packaging/build-dist.sh (tarball, .deb and
#              .rpm with the pinned cargo-deb/cargo-generate-rpm, version-less
#              copies and SHA256SUMS in <target>/dist), then: checksums,
#              package metadata and contents (no maintainer scripts, nothing
#              under /etc, the unit installed but not enabled), the
#              validators, every dosi-tracker command in the README parses,
#              and a rootless install and uninstall from the tarball
#   all        fmt clippy test harness installer release (package is separate)
#
# An unknown stage exits 2 before anything runs.

set -euo pipefail

CRATE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
cd "${CRATE_DIR}"

TARGET_DIR="${CARGO_TARGET_DIR:-${CRATE_DIR}/target}"
RELEASE_BINS=(dosi-tracker dosi-tracker-gui)

# The only shared objects a release binary may need: the GUI dlopens its
# libraries at run time and everything else is linked statically.
LDD_ALLOWLIST_RE='^(linux-vdso\.so\.1|libc\.so\.6|libm\.so\.6|libgcc_s\.so\.1|/lib64/ld-linux-x86-64\.so\.2|ld-linux-x86-64\.so\.2)$'

log() {
  printf '\n==> %s\n' "$*"
}

die() {
  printf 'ERROR: %s\n' "$*" >&2
  exit 1
}

stage_fmt() {
  log "fmt"
  cargo fmt --check
}

stage_clippy() {
  log "clippy (default features)"
  cargo clippy --locked --all-targets -- -D warnings
  log "clippy (--no-default-features)"
  cargo clippy --locked --all-targets --no-default-features -- -D warnings
  log "clippy (--all-features)"
  cargo clippy --locked --all-targets --all-features -- -D warnings
}

stage_test() {
  log "test (default features)"
  cargo test --locked
  log "test (--features test-support)"
  cargo test --locked --features test-support
}

stage_harness() {
  log "harness (Xvfb + private buses, ignored tests)"
  bash ci/xvfb-session.sh cargo test --locked --features test-support -- --ignored --test-threads=1
}

stage_installer() {
  log "installer lifecycle"
  if [ "$(id -u)" -eq 0 ]; then
    die "the installer suite must run as a non-root user (the installer refuses root)"
  fi
  # The suite builds the release binaries (LTO): bound the parallel links.
  CARGO_BUILD_JOBS="$(lto_jobs)" bash tests/test_installer_lifecycle.sh
}

# A fresh, empty HOME/XDG tree with nothing of the caller's session in it.
isolated_env() {
  local home="$1"
  shift
  env -i \
    PATH="/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin" \
    LANG=C.UTF-8 \
    HOME="${home}" \
    XDG_CONFIG_HOME="${home}/.config" \
    XDG_DATA_HOME="${home}/.local/share" \
    XDG_STATE_HOME="${home}/.local/state" \
    "$@"
}

check_ldd_allowlist() {
  local bin="$1"
  local output lib unexpected=""
  output="$(ldd "${bin}")" || die "ldd failed for ${bin}"
  printf '%s\n' "${output}"
  while read -r lib _; do
    [ -n "${lib}" ] || continue
    if ! [[ "${lib}" =~ ${LDD_ALLOWLIST_RE} ]]; then
      unexpected="${unexpected} ${lib}"
    fi
  done <<<"${output}"
  if [ -n "${unexpected}" ]; then
    die "$(basename "${bin}") links libraries outside the allowlist:${unexpected}"
  fi
}

check_outbox_status_fails_closed() {
  local bin="$1"
  local home missing_db
  home="$(mktemp -d)"
  missing_db="${home}/missing_dosi_outbox_${RANDOM}_$$.db"
  rm -f "${missing_db}"
  if isolated_env "${home}" "${bin}" outbox status --db "${missing_db}" \
    --account-id "00000000-0000-0000-0000-000000000001" \
    --workspace-id "00000000-0000-0000-0000-000000000002" --json; then
    die "outbox status succeeded unexpectedly on a non-existent DB"
  fi
  if [ -e "${missing_db}" ]; then
    die "outbox status created the database file"
  fi
  rm -rf "${home}"
}

# Parallel jobs for the release (LTO) builds: each LTO link of a binary that
# carries the GUI takes about 2 GiB, so a many-core host with little memory
# per core would have the kernel kill rustc. CARGO_BUILD_JOBS wins when set.
lto_jobs() {
  if [ -n "${CARGO_BUILD_JOBS:-}" ]; then
    echo "${CARGO_BUILD_JOBS}"
    return
  fi
  local mem_kb cpus jobs
  mem_kb="$(awk '/^MemAvailable:/ {print $2}' /proc/meminfo 2>/dev/null || echo 0)"
  cpus="$(nproc 2>/dev/null || echo 1)"
  jobs=$((mem_kb / (2 * 1024 * 1024)))
  [ "${jobs}" -ge 1 ] || jobs=1
  [ "${jobs}" -le "${cpus}" ] || jobs="${cpus}"
  echo "${jobs}"
}

stage_release() {
  local jobs
  jobs="$(lto_jobs)"
  log "release build (${jobs} jobs)"
  cargo build --locked --release --bins -j "${jobs}"

  log "release-mode tests (${jobs} jobs)"
  cargo test --locked --release -j "${jobs}"

  local name bin
  for name in "${RELEASE_BINS[@]}"; do
    bin="${TARGET_DIR}/release/${name}"
    [ -x "${bin}" ] || die "missing release binary ${bin}"
    log "ldd allowlist: ${name}"
    check_ldd_allowlist "${bin}"
    log "--version: ${name}"
    "${bin}" --version | grep -E "^${name} [0-9]+\.[0-9]+\.[0-9]+" \
      || die "${name} --version does not print '${name} <version>'"
  done

  local cli="${TARGET_DIR}/release/dosi-tracker"
  local home
  home="$(mktemp -d)"
  log "doctor --json in an empty HOME"
  isolated_env "${home}" "${cli}" doctor --json | python3 -m json.tool >/dev/null \
    || die "doctor --json did not print one JSON document"
  if [ -e "${home}/.config/dosi-tracker" ] || [ -e "${home}/.local/share/dosi-tracker" ]; then
    die "doctor --json created files in HOME"
  fi
  rm -rf "${home}"

  log "outbox status fails closed on a missing DB"
  check_outbox_status_fails_closed "${cli}"

  log "no test hooks in the release binary"
  if "${cli}" daemon --help | grep -Eiq 'fake|test'; then
    die "daemon --help shows a test flag"
  fi
  if grep -aq 'DOSI_TEST_' "${cli}"; then
    die "test-support is compiled into the release binary"
  fi

  log "passwords on argv are refused, status without an agent creates nothing"
  check_cli_fails_closed "${cli}"
}

# `login --password x` exits 2 without echoing x; `status --no-start` exits
# 3; neither leaves a file behind.
check_cli_fails_closed() {
  local bin="$1"
  local home code out
  home="$(mktemp -d)"
  chmod 700 "${home}"
  touch "${home}/.stamp"
  sleep 1
  set +e
  out="$(isolated_env "${home}" XDG_RUNTIME_DIR="${home}" "${bin}" login --password release-canary-x 2>&1)"
  code=$?
  set -e
  [ "${code}" -eq 2 ] || die "login --password exited ${code}, not 2"
  if printf '%s' "${out}" | grep -q 'release-canary-x'; then
    die "login --password echoed the password"
  fi
  set +e
  isolated_env "${home}" "${bin}" status --no-start >/dev/null 2>&1
  code=$?
  set -e
  [ "${code}" -eq 3 ] || die "status --no-start exited ${code}, not 3"
  if [ -n "$(find "${home}" -mindepth 1 -newer "${home}/.stamp")" ]; then
    die "login/status created files: $(find "${home}" -mindepth 1 -newer "${home}/.stamp")"
  fi
  rm -rf "${home}"
}

# Packaging tools, pinned and built from crates.io with --locked into the cargo
# registry volume (so CI and local runs reuse them).
CARGO_DEB_VERSION="3.8.0"
CARGO_GENERATE_RPM_VERSION="0.21.1"

install_packaging_tools() {
  local root="${PACKAGING_TOOLS_ROOT:-${CARGO_HOME:-${HOME}/.cargo}/registry/tools}"
  export PATH="${root}/bin:${PATH}"
  local tool name version
  for tool in "cargo-deb@${CARGO_DEB_VERSION}" "cargo-generate-rpm@${CARGO_GENERATE_RPM_VERSION}"; do
    name="${tool%@*}"
    version="${tool#*@}"
    if [ "$("${root}/bin/${name}" --version 2>/dev/null)" = "${name} ${version}" ]; then
      continue
    fi
    log "installing ${tool} into ${root}"
    CARGO_TARGET_DIR="${TARGET_DIR}/packaging-tools" \
      cargo install --locked --force --root "${root}" "${tool}"
  done
}

# Every `dosi-tracker ...` command in the README's shell blocks names a real
# subcommand path: walk it with --help, descending only while the current
# command has subcommands.
check_readme_commands() {
  local bin="$1"
  python3 - "${bin}" README.md <<'PY'
import re
import shlex
import subprocess
import sys

binary, readme = sys.argv[1], sys.argv[2]
GLOBAL_FLAGS = {"--json", "--no-start", "-q", "--quiet", "-v", "--verbose", "--no-color"}
lines, in_block, current = [], False, ""
for raw in open(readme, encoding="utf-8"):
    fence = re.match(r"^\s*```(\w*)", raw)
    if fence:
        in_block = not in_block and fence.group(1) in ("bash", "sh", "shell", "console")
        continue
    if not in_block:
        continue
    text = raw.rstrip("\n")
    if text.rstrip().endswith("\\"):
        current += text.rstrip()[:-1] + " "
        continue
    lines.append(current + text)
    current = ""

checked = 0
failures = []
for line in lines:
    lexer = shlex.shlex(line, posix=True, punctuation_chars=True)
    lexer.whitespace_split = True
    lexer.commenters = "#"
    try:
        tokens = list(lexer)
    except ValueError as e:
        failures.append("%s: %s" % (line, e))
        continue
    segment = []
    for token in tokens + [";"]:
        if token in ("|", "||", "&&", ";", "&"):
            if segment and segment[0].split("/")[-1] == "dosi-tracker":
                path = []
                for word in segment[1:]:
                    if word in GLOBAL_FLAGS:
                        continue
                    if word.startswith("-"):
                        break
                    help_text = subprocess.run(
                        [binary] + path + ["--help"], capture_output=True, text=True
                    ).stdout
                    if "Commands:" not in help_text:
                        break
                    path.append(word)
                result = subprocess.run([binary] + path + ["--help"], capture_output=True, text=True)
                checked += 1
                if result.returncode != 0:
                    failures.append("%s  ->  dosi-tracker %s --help exited %d: %s" % (
                        " ".join(segment), " ".join(path), result.returncode, result.stderr.strip()))
            segment = []
        else:
            segment.append(token)
if failures:
    print("README commands that do not parse:", *failures, sep="\n  ", file=sys.stderr)
    sys.exit(1)
if checked < 20:
    print("only %d dosi-tracker commands found in README.md" % checked, file=sys.stderr)
    sys.exit(1)
print("README: %d dosi-tracker commands checked" % checked)
PY
}

# The installed file list of a package ($1, one path per line) has exactly the
# design 15.2 /usr layout and nothing under /etc.
check_package_files() {
  local list="$1" kind="$2"
  local app="io.github.dosibridge.DosiTracker" path size
  local zsh_dir="/usr/share/zsh/vendor-completions"
  [ "${kind}" = "rpm" ] && zsh_dir="/usr/share/zsh/site-functions"
  for path in /usr/bin/dosi-tracker /usr/bin/dosi-tracker-gui \
    /usr/lib/systemd/user/dosi-tracker.service \
    "/usr/share/applications/${app}.desktop" "/usr/share/metainfo/${app}.metainfo.xml" \
    "/usr/share/icons/hicolor/scalable/apps/${app}.svg" \
    /usr/share/bash-completion/completions/dosi-tracker "${zsh_dir}/_dosi-tracker" \
    /usr/share/fish/vendor_completions.d/dosi-tracker.fish; do
    grep -qx "${path}" "${list}" || die "${kind}: ${path} is missing"
  done
  for size in 16x16 22x22 24x24 32x32 48x48 64x64 128x128 256x256 512x512; do
    grep -qx "/usr/share/icons/hicolor/${size}/apps/${app}.png" "${list}" || die "${kind}: ${size} icon is missing"
  done
  if grep -E '^/etc(/|$)' "${list}"; then
    die "${kind}: installs files under /etc (no /etc/xdg/autostart, no system config)"
  fi
  if grep -E '\.wants/|/systemd/system/' "${list}"; then
    die "${kind}: enables a unit or installs a system unit"
  fi
}

stage_package() {
  install_packaging_tools
  local jobs
  jobs="$(lto_jobs)"
  log "release build (${jobs} jobs)"
  cargo build --locked --release --bins -j "${jobs}"

  log "packaging/build-dist.sh"
  bash packaging/build-dist.sh

  local dist="${DIST_DIR:-${TARGET_DIR}/dist}"
  local version
  version="$("${TARGET_DIR}/release/dosi-tracker" --version | awk '{print $2}')"
  local tarball="dosi-tracker-${version}-x86_64-linux-gnu.tar.gz"
  local deb="dosi-tracker_${version}-1_amd64.deb"
  local rpm="dosi-tracker-${version}-1.x86_64.rpm"

  log "checksums"
  local f
  for f in "${tarball}" "${deb}" "${rpm}" dosi-tracker-linux-x86_64.tar.gz dosi-tracker_amd64.deb dosi-tracker.x86_64.rpm; do
    [ -s "${dist}/${f}" ] || die "missing artifact ${f}"
    grep -q "  ${f}\$" "${dist}/SHA256SUMS" || die "${f} is not in SHA256SUMS"
  done
  (cd "${dist}" && sha256sum -c SHA256SUMS)
  cmp "${dist}/${tarball}" "${dist}/dosi-tracker-linux-x86_64.tar.gz"
  cmp "${dist}/${deb}" "${dist}/dosi-tracker_amd64.deb"
  cmp "${dist}/${rpm}" "${dist}/dosi-tracker.x86_64.rpm"

  local scratch
  scratch="$(mktemp -d)"
  log "deb: metadata and contents"
  dpkg-deb --info "${dist}/${deb}"
  [ "$(dpkg-deb --field "${dist}/${deb}" Package)" = "dosi-tracker" ] || die "deb Package is not dosi-tracker"
  [ "$(dpkg-deb --field "${dist}/${deb}" Version)" = "${version}-1" ] || die "deb Version is not ${version}-1"
  dpkg-deb --field "${dist}/${deb}" Depends | grep -q 'libc6' || die "deb Depends lacks libc6"
  dpkg-deb --field "${dist}/${deb}" Recommends | grep -q 'xdg-desktop-portal' || die "deb Recommends is incomplete"
  dpkg-deb --field "${dist}/${deb}" Suggests | grep -q 'gnome-shell-extension-appindicator' || die "deb Suggests is incomplete"
  dpkg-deb --control "${dist}/${deb}" "${scratch}/control"
  for f in preinst postinst prerm postrm config triggers; do
    [ ! -e "${scratch}/control/${f}" ] || die "the deb has a maintainer script: ${f}"
  done
  dpkg-deb --fsys-tarfile "${dist}/${deb}" | tar -t | sed 's|^\./|/|' | grep -v '/$' | sort > "${scratch}/deb.files"
  check_package_files "${scratch}/deb.files" deb

  log "rpm: metadata and contents"
  rpm -qip "${dist}/${rpm}"
  [ "$(rpm -qp --queryformat '%{NAME} %{VERSION}-%{RELEASE}' "${dist}/${rpm}")" = "dosi-tracker ${version}-1" ] \
    || die "rpm name/version is not dosi-tracker ${version}-1"
  [ -z "$(rpm -qp --scripts "${dist}/${rpm}")" ] || die "the rpm has scriptlets"
  rpm -qp --recommends "${dist}/${rpm}" | grep -q 'mesa-libEGL' || die "rpm Recommends is incomplete"
  rpm -qp --requires "${dist}/${rpm}" | grep -q 'libc.so.6' || die "rpm Requires lacks libc"
  rpm -qlp "${dist}/${rpm}" | sort > "${scratch}/rpm.files"
  check_package_files "${scratch}/rpm.files" rpm

  log "validators"
  desktop-file-validate "packaging/desktop/io.github.dosibridge.DosiTracker.desktop"
  desktop-file-validate "${TARGET_DIR}/release/dist-assets/usr/io.github.dosibridge.DosiTracker.desktop"
  appstreamcli validate --no-net packaging/metainfo/io.github.dosibridge.DosiTracker.metainfo.xml
  shellcheck -x scripts/*.sh ci/*.sh packaging/*.sh tests/test_installer_lifecycle.sh

  log "README commands parse"
  check_readme_commands "${TARGET_DIR}/release/dosi-tracker"

  log "tarball: rootless install and uninstall"
  tar -xzf "${dist}/${tarball}" -C "${scratch}"
  local tree="${scratch}/dosi-tracker-${version}"
  cmp "${tree}/install.sh" scripts/install.sh || die "the tarball's install.sh differs from scripts/install.sh"
  if tar -tzf "${dist}/${tarball}" | grep -Eq '(^|/)(etc|\.git|target)/'; then
    die "the tarball carries files it must not"
  fi
  local home="${scratch}/home"
  mkdir -p "${home}"
  isolated_env "${home}" bash "${tree}/install.sh" >/dev/null
  isolated_env "${home}" "${home}/.local/bin/dosi-tracker" --version
  isolated_env "${home}" "${home}/.local/bin/dosi-tracker-gui" --version
  [ -f "${home}/.local/share/systemd/user/dosi-tracker.service" ] || die "the tarball install has no user unit"
  [ ! -e "${home}/.config/autostart" ] || die "the tarball install wrote an autostart entry"
  isolated_env "${home}" bash "${home}/.local/lib/dosi-tracker/install/uninstall.sh" --yes >/dev/null
  if [ -n "$(find "${home}/.local" -type f)" ]; then
    die "files left after uninstall: $(find "${home}/.local" -type f)"
  fi
  rm -rf "${scratch}"
}

run_stage() {
  case "$1" in
    fmt) stage_fmt ;;
    clippy) stage_clippy ;;
    test) stage_test ;;
    harness) stage_harness ;;
    installer) stage_installer ;;
    release) stage_release ;;
    package) stage_package ;;
    all)
      stage_fmt
      stage_clippy
      stage_test
      stage_harness
      stage_installer
      stage_release
      ;;
  esac
}

main() {
  if [ "$#" -eq 0 ]; then
    echo "usage: bash ci/check.sh <fmt|clippy|test|harness|installer|release|package|all>..." >&2
    exit 2
  fi
  local stage
  for stage in "$@"; do
    case "${stage}" in
      fmt | clippy | test | harness | installer | release | package | all) ;;
      *)
        echo "ci/check.sh: unknown stage '${stage}'" >&2
        exit 2
        ;;
    esac
  done
  for stage in "$@"; do
    run_stage "${stage}"
  done
  log "ci/check.sh: $* passed"
}

main "$@"
