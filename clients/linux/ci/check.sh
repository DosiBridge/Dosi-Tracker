#!/usr/bin/env bash
#
# Linux agent CI stages. Runs inside the ci/Dockerfile image (locally through
# Docker, in CI through .github/workflows/linux.yml) from clients/linux:
#
#   bash ci/check.sh all
#   bash ci/check.sh fmt clippy test installer release
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
#   all        fmt clippy test harness installer release
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
  bash tests/test_installer_lifecycle.sh
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

stage_release() {
  log "release build"
  cargo build --locked --release --bins

  log "release-mode tests"
  cargo test --locked --release

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

run_stage() {
  case "$1" in
    fmt) stage_fmt ;;
    clippy) stage_clippy ;;
    test) stage_test ;;
    harness) stage_harness ;;
    installer) stage_installer ;;
    release) stage_release ;;
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
    echo "usage: bash ci/check.sh <fmt|clippy|test|harness|installer|release|all>..." >&2
    exit 2
  fi
  local stage
  for stage in "$@"; do
    case "${stage}" in
      fmt | clippy | test | harness | installer | release | all) ;;
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
