#!/usr/bin/env bash
#
# Lifecycle test suite for the Dosi Tracker Linux rootless installer (install.sh v2).
#
# Kept from the foundation installer (now against manifest format 2, and against the
# format 1 manifest where migration is involved):
#  1. Fresh rootless install to a temporary prefix
#  2. Foreign/unmanaged file conflict refusal (preserves the foreign file)
#  3. Malformed JSON manifest refusal (fails closed, preserves bytes)
#  4. Duplicate keys manifest refusal
#  5. Invalid owned_files structure refusal
#  6. Mismatched prefix/binary manifest refusal
#  7. Sibling prefix and dot-dot traversal refusal
#  8. Ancestor and explicit symlink escape refusal
#  9. Rollback refuses to overwrite an unmanaged foreign destination
# 10. Rollback refuses a foreign-prefix or mismatched backup
# 11. Rollback restores the complete managed state
# 12. Injected metadata publication failure rolls everything back
# 13. Injected post-activation smoke failure rolls everything back
# 14. Uninstall refuses a corrupted manifest
# 15. Uninstall preserves user config and unrelated sentinels
# 16. Idempotent uninstall
# 17. Real compiled binary install and doctor smoke test
# 18. Extra in-prefix sentinel, traversal, symlink-parent, newline/whitespace entries refused
# 19. Update and rollback refuse unauthorized owned_files
# New in v2:
# 20. Inventory confinement: every owned file is inside the prefix and in the inventory,
#     nothing is written outside it; desktop entry with absolute Exec; completions
# 21. User unit only for ~/.local, never enabled; no autostart entry ever written
# 22. Format 1 manifest migration (and rollback to the migrated install)
# 23. Update stops a running agent gracefully and restarts it; a failed update restarts
#     the previous one
# 24. Tarball mode and the installed uninstaller copy
# 25. Root refused before any change
# 26. --server/--dashboard: invalid URL exits 2 before any change; valid ones are saved
# 27. Plain uninstall keeps config, consent and data; --purge removes them (dosi-tracker
#     reset), and needs --yes without a terminal
# 28. `dosi-tracker uninstall` runs the installed uninstaller
#
# Tests that need their own HOME change it inside a ( ... ) subshell on purpose.
# shellcheck disable=SC2030,SC2031

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
LINUX_CRATE_DIR="$(cd "${SCRIPT_DIR}/.." && pwd -P)"
INSTALLER="${LINUX_CRATE_DIR}/scripts/install.sh"
APP_ID="io.github.dosibridge.DosiTracker"
META_SUBDIR="lib/dosi-tracker/install"

# Nothing of the caller's session leaks into the installer or the programs it runs.
export CARGO_HOME="${CARGO_HOME:-${HOME}/.cargo}"
export RUSTUP_HOME="${RUSTUP_HOME:-${HOME}/.rustup}"
SUITE_HOME="$(mktemp -d)"
export HOME="${SUITE_HOME}/home"
mkdir -p "${HOME}"
unset XDG_CONFIG_HOME XDG_DATA_HOME XDG_STATE_HOME XDG_RUNTIME_DIR DOSI_SOCKET \
  DBUS_SESSION_BUS_ADDRESS DISPLAY WAYLAND_DISPLAY PREFIX DOSI_INSTALL_SOURCE_BIN || true
trap 'rm -rf "${SUITE_HOME}"' EXIT

FAILED_TESTS=0
PASSED_TESTS=0

run_test() {
  local test_name="$1"
  shift
  # INSTALLER_TEST_FILTER=<substring> runs only the matching tests (development aid).
  if [ -n "${INSTALLER_TEST_FILTER:-}" ] && [[ "${test_name}" != *"${INSTALLER_TEST_FILTER}"* ]]; then
    return 0
  fi
  printf "RUNNING: %s... " "${test_name}"
  if ( "$@" ); then
    printf "OK\n"
    PASSED_TESTS=$((PASSED_TESTS + 1))
  else
    printf "FAILED\n"
    FAILED_TESTS=$((FAILED_TESTS + 1))
  fi
}

# A synthetic dosi-tracker. It records the service calls it gets in <state>/calls
# and pretends to be running while <state>/running exists.
create_fake_executable() {
  local target_path="$1"
  local version="${2:-0.1.0}"
  local state="${3:-/nonexistent}"
  cat > "${target_path}" <<EOF
#!/usr/bin/env bash
state="${state}"
[ "\$1" = "--no-start" ] && shift
[ "\$1" = "--json" ] && shift
case "\$1 \${2:-}" in
  "--version "*) echo "dosi-tracker ${version}" ;;
  "--help "*) echo "dosi-tracker help" ;;
  "doctor "*) echo '{"component":"dosi-tracker","version":"${version}","status":"ok"}' ;;
  "completions "*) echo "# ${version} completion for \$2" ;;
  "service status")
    echo "status ${version}" >> "\${state}/calls" 2>/dev/null || true
    [ -e "\${state}/running" ] ;;
  "service stop")
    echo "stop ${version}" >> "\${state}/calls"
    rm -f "\${state}/running" ;;
  "service start")
    echo "start ${version}" >> "\${state}/calls"
    [ ! -e "\${state}/fail-start-${version}" ] && touch "\${state}/running" ;;
  "service disable")
    echo "disable ${version}" >> "\${state}/calls" 2>/dev/null || true ;;
  "queue status") echo '{"ok":true,"result":{"pending":0,"parked":0}}' ;;
  *) echo "Unknown command: \$1"; exit 1 ;;
esac
EOF
  chmod 755 "${target_path}"
}

create_fake_gui() {
  local target_path="$1"
  local version="${2:-0.1.0}"
  cat > "${target_path}" <<EOF
#!/usr/bin/env bash
[ "\$1" = "--version" ] && { echo "dosi-tracker-gui ${version}"; exit 0; }
exit 1
EOF
  chmod 755 "${target_path}"
}

# The real release binaries, built once for the tests that need the real CLI.
REAL_BIN=""
REAL_GUI=""
ensure_real_binaries() {
  if [ -n "${REAL_BIN}" ]; then
    return 0
  fi
  cargo build --locked --release --manifest-path "${LINUX_CRATE_DIR}/Cargo.toml" --bins >&2
  local target_dir="${CARGO_TARGET_DIR:-${LINUX_CRATE_DIR}/target}"
  REAL_BIN="${target_dir}/release/dosi-tracker"
  REAL_GUI="${target_dir}/release/dosi-tracker-gui"
  [ -x "${REAL_BIN}" ] && [ -x "${REAL_GUI}" ]
}

meta_dir() { printf '%s' "$1/${META_SUBDIR}"; }

# Write a format 2 manifest for prefix $1 with the owned files $3... into file $2.
write_manifest_v2() {
  local prefix="$1" out="$2"
  shift 2
  local owned="" item
  for item in "$@"; do
    owned="${owned}${owned:+,
}    \"${item}\""
  done
  cat > "${out}" <<EOF
{
  "format_version": 2,
  "component": "dosi-tracker",
  "version": "0.1.0",
  "installed_at": "2026-10-22T00:00:00Z",
  "source": "binary",
  "prefix": "${prefix}",
  "bin_dir": "${prefix}/bin",
  "metadata_dir": "${prefix}/${META_SUBDIR}",
  "binary_path": "${prefix}/bin/dosi-tracker",
  "owned_files": [
${owned}
  ],
  "previous_version": null
}
EOF
}

# A format 1 (foundation installer) manifest for prefix $1.
write_manifest_v1() {
  local prefix="$1" version="${2:-0.0.9}"
  mkdir -p "${prefix}/share/dosi-tracker"
  cat > "${prefix}/share/dosi-tracker/manifest.json" <<EOF
{
  "format_version": 1,
  "component": "dosi-tracker-linux-diagnostic-foundation",
  "version": "${version}",
  "installed_at": "2026-10-22T00:00:00Z",
  "prefix": "${prefix}",
  "bin_dir": "${prefix}/bin",
  "data_dir": "${prefix}/share/dosi-tracker",
  "binary_path": "${prefix}/bin/dosi-tracker",
  "owned_files": [
    "${prefix}/bin/dosi-tracker",
    "${prefix}/share/dosi-tracker/manifest.json",
    "${prefix}/share/dosi-tracker/current_version"
  ],
  "previous_version": null
}
EOF
  echo "${version}" > "${prefix}/share/dosi-tracker/current_version"
}

# 1. Fresh install to an isolated prefix
test_fresh_install_to_isolated_temp_prefix() {
  local temp_dir
  temp_dir="$(mktemp -d)"
  local fake_src="${temp_dir}/fake_bin"
  create_fake_executable "${fake_src}" "0.1.0"

  local prefix="${temp_dir}/opt/dosi"
  bash "${INSTALLER}" install --prefix "${prefix}" --source-bin "${fake_src}" >/dev/null

  local bin_path="${prefix}/bin/dosi-tracker"
  local manifest
  manifest="$(meta_dir "${prefix}")/manifest.json"
  local out
  out="$("${bin_path}" --version)"
  local ok=1
  [ -f "${bin_path}" ] || ok=0
  grep -q '"format_version": 2' "${manifest}" || ok=0
  grep -q '"component": "dosi-tracker"' "${manifest}" || ok=0
  grep -q '"version": "0.1.0"' "${manifest}" || ok=0
  [ "$(cat "$(meta_dir "${prefix}")/current_version")" = "0.1.0" ] || ok=0
  [ -x "$(meta_dir "${prefix}")/uninstall.sh" ] || ok=0
  # Nothing in the foundation installer's metadata location.
  [ ! -e "${prefix}/share/dosi-tracker" ] || ok=0
  rm -rf "${temp_dir}"
  [ "${ok}" = "1" ] && [[ "${out}" == *"0.1.0"* ]]
}

# 2. Foreign / unmanaged file conflict refusal
test_foreign_file_conflict_refusal() {
  local temp_dir
  temp_dir="$(mktemp -d)"
  local prefix="${temp_dir}/opt/dosi"
  mkdir -p "${prefix}/bin" "${prefix}/share/icons/hicolor/48x48/apps"
  echo "foreign proprietary payload" > "${prefix}/bin/dosi-tracker"

  local fake_src="${temp_dir}/fake_bin"
  create_fake_executable "${fake_src}" "0.1.0"

  local failed="0"
  if bash "${INSTALLER}" install --prefix "${prefix}" --source-bin "${fake_src}" >/dev/null 2>&1; then
    failed="1"
  fi
  local content
  content="$(cat "${prefix}/bin/dosi-tracker")"

  # A foreign file at any other inventory path is refused too, before any change.
  rm -f "${prefix}/bin/dosi-tracker"
  echo "foreign icon" > "${prefix}/share/icons/hicolor/48x48/apps/${APP_ID}.png"
  if bash "${INSTALLER}" install --prefix "${prefix}" --source-bin "${fake_src}" >/dev/null 2>&1; then
    failed="1"
  fi
  local icon untouched=1
  icon="$(cat "${prefix}/share/icons/hicolor/48x48/apps/${APP_ID}.png")"
  [ ! -e "${prefix}/bin/dosi-tracker" ] && [ ! -e "$(meta_dir "${prefix}")/manifest.json" ] || untouched=0

  rm -rf "${temp_dir}"
  [ "${failed}" = "0" ] && [ "${content}" = "foreign proprietary payload" ] \
    && [ "${icon}" = "foreign icon" ] && [ "${untouched}" = "1" ]
}

# Writes a broken manifest of kind $3 into $2 (for prefix $1).
write_broken_manifest() {
  local prefix="$1" out="$2" kind="$3"
  local v2_meta="${prefix}/${META_SUBDIR}"
  local head
  case "${out}" in
    *"/${META_SUBDIR}/"*)
      head="\"format_version\": 2,
  \"component\": \"dosi-tracker\",
  \"source\": \"binary\",
  \"metadata_dir\": \"${v2_meta}\","
      ;;
    *)
      head="\"format_version\": 1,
  \"component\": \"dosi-tracker-linux-diagnostic-foundation\",
  \"data_dir\": \"${prefix}/share/dosi-tracker\","
      ;;
  esac
  case "${kind}" in
    missing_comma)
      cat > "${out}" <<EOF
{
  "format_version": 2
  "component": "dosi-tracker",
  "prefix": "${prefix}",
  "owned_files": ["${prefix}/bin/dosi-tracker"]
}
EOF
      ;;
    trailing_garbage)
      cat > "${out}" <<EOF
{
  ${head}
  "version": "0.1.0",
  "installed_at": "2026-10-22T00:00:00Z",
  "prefix": "${prefix}",
  "bin_dir": "${prefix}/bin",
  "binary_path": "${prefix}/bin/dosi-tracker",
  "owned_files": ["${prefix}/bin/dosi-tracker"],
  "previous_version": null
} TRAILING_GARBAGE_BYTES
EOF
      ;;
    duplicate_key)
      cat > "${out}" <<EOF
{
  ${head}
  "version": "0.1.0",
  "version": "0.2.0",
  "installed_at": "2026-10-22T00:00:00Z",
  "prefix": "${prefix}",
  "bin_dir": "${prefix}/bin",
  "binary_path": "${prefix}/bin/dosi-tracker",
  "owned_files": ["${prefix}/bin/dosi-tracker"],
  "previous_version": null
}
EOF
      ;;
    empty_owned|owned_not_array|binary_not_owned)
      local owned='[]'
      [ "${kind}" = "owned_not_array" ] && owned='"not_an_array"'
      [ "${kind}" = "binary_not_owned" ] && owned="[\"${prefix}/share/dosi-tracker/other_file\"]"
      cat > "${out}" <<EOF
{
  ${head}
  "version": "0.1.0",
  "installed_at": "2026-10-22T00:00:00Z",
  "prefix": "${prefix}",
  "bin_dir": "${prefix}/bin",
  "binary_path": "${prefix}/bin/dosi-tracker",
  "owned_files": ${owned},
  "previous_version": null
}
EOF
      ;;
    foreign_prefix)
      local other="/another/unrelated/prefix"
      cat > "${out}" <<EOF
{
  ${head//${prefix}/${other}}
  "version": "0.1.0",
  "installed_at": "2026-10-22T00:00:00Z",
  "prefix": "${other}",
  "bin_dir": "${other}/bin",
  "binary_path": "${other}/bin/dosi-tracker",
  "owned_files": ["${other}/bin/dosi-tracker"],
  "previous_version": null
}
EOF
      ;;
  esac
}

# Install refuses with an existing binary and a manifest of kind $1 at either the
# format 2 location or the foundation (format 1) location; the binary is kept.
refuses_broken_manifest_kinds() {
  local kind location ok=1
  for kind in "$@"; do
    for location in v2 v1; do
      local temp_dir
      temp_dir="$(mktemp -d)"
      local prefix="${temp_dir}/opt/dosi"
      local manifest_dir="${prefix}/${META_SUBDIR}"
      [ "${location}" = "v1" ] && manifest_dir="${prefix}/share/dosi-tracker"
      mkdir -p "${prefix}/bin" "${manifest_dir}"
      echo "existing payload" > "${prefix}/bin/dosi-tracker"
      write_broken_manifest "${prefix}" "${manifest_dir}/manifest.json" "${kind}"
      local before
      before="$(cat "${manifest_dir}/manifest.json")"
      local fake_src="${temp_dir}/fake_bin"
      create_fake_executable "${fake_src}" "0.1.0"
      if bash "${INSTALLER}" install --prefix "${prefix}" --source-bin "${fake_src}" >/dev/null 2>&1; then
        echo "install accepted a ${kind} manifest at ${location}" >&2
        ok=0
      fi
      if bash "${INSTALLER}" uninstall --prefix "${prefix}" >/dev/null 2>&1; then
        echo "uninstall accepted a ${kind} manifest at ${location}" >&2
        ok=0
      fi
      [ "$(cat "${prefix}/bin/dosi-tracker")" = "existing payload" ] || ok=0
      [ "$(cat "${manifest_dir}/manifest.json")" = "${before}" ] || ok=0
      rm -rf "${temp_dir}"
    done
  done
  [ "${ok}" = "1" ]
}

# 3. Malformed JSON manifest refusal (missing comma, trailing garbage)
test_malformed_manifest_refusal() {
  refuses_broken_manifest_kinds missing_comma trailing_garbage
}

# 4. Duplicate keys
test_duplicate_keys_manifest_refusal() {
  refuses_broken_manifest_kinds duplicate_key
}

# 5. Invalid owned_files structure
test_invalid_owned_files_refusal() {
  refuses_broken_manifest_kinds empty_owned owned_not_array binary_not_owned
}

# 6. Mismatched prefix/binary manifest
test_mismatched_prefix_manifest_refusal() {
  refuses_broken_manifest_kinds foreign_prefix
}

# 7. Sibling prefix and dot-dot traversal refusal
test_sibling_prefix_and_dot_dot_escape_refusal() {
  local temp_dir
  temp_dir="$(mktemp -d)"
  local prefix="${temp_dir}/opt/dosi"
  local fake_src="${temp_dir}/fake_bin"
  create_fake_executable "${fake_src}" "0.1.0"

  local passed=1
  if bash "${INSTALLER}" install --prefix "${prefix}" --bin-dir "${temp_dir}/opt/dosi-sibling/bin" --source-bin "${fake_src}" >/dev/null 2>&1; then
    passed=0
  fi
  if bash "${INSTALLER}" install --prefix "${prefix}" --bin-dir "${prefix}/sub/../../outside/bin" --source-bin "${fake_src}" >/dev/null 2>&1; then
    passed=0
  fi
  # Refusals happen before anything is created.
  [ ! -e "${temp_dir}/opt/dosi-sibling" ] && [ ! -e "${temp_dir}/opt/outside" ] && [ ! -e "${prefix}" ] || passed=0
  rm -rf "${temp_dir}"
  [ "${passed}" = "1" ]
}

# 8. Ancestor and explicit symlink escape refusal
test_ancestor_and_explicit_symlink_escape_refusal() {
  local temp_dir
  temp_dir="$(mktemp -d)"
  local prefix="${temp_dir}/opt/dosi"
  local outside_dir="${temp_dir}/outside"
  mkdir -p "${prefix}" "${outside_dir}"
  ln -s "${outside_dir}" "${prefix}/bin" 2>/dev/null || true
  if [ ! -L "${prefix}/bin" ]; then
    rm -rf "${temp_dir}"
    return 0 # no directory symlinks without elevation on this host; skip cleanly
  fi
  local fake_src="${temp_dir}/fake_bin"
  create_fake_executable "${fake_src}" "0.1.0"

  local passed=0
  if ! bash "${INSTALLER}" install --prefix "${prefix}" --bin-dir "${prefix}/bin" --source-bin "${fake_src}" >/dev/null 2>&1; then
    passed=1
  fi
  # The metadata directory is checked the same way.
  rm -f "${prefix}/bin"
  mkdir -p "${prefix}/lib"
  ln -s "${outside_dir}" "${prefix}/lib/dosi-tracker"
  if bash "${INSTALLER}" install --prefix "${prefix}" --source-bin "${fake_src}" >/dev/null 2>&1; then
    passed=0
  fi
  [ -z "$(ls -A "${outside_dir}")" ] || passed=0
  rm -rf "${temp_dir}"
  [ "${passed}" = "1" ]
}

# 9. Rollback refuses to overwrite an unmanaged foreign destination
test_rollback_refuses_to_overwrite_unmanaged_foreign_destination() {
  local temp_dir
  temp_dir="$(mktemp -d)"
  local prefix="${temp_dir}/opt/dosi"
  create_fake_executable "${temp_dir}/v1" "0.1.0"
  create_fake_executable "${temp_dir}/v2" "0.2.0"
  bash "${INSTALLER}" install --prefix "${prefix}" --source-bin "${temp_dir}/v1" >/dev/null
  bash "${INSTALLER}" update --prefix "${prefix}" --source-bin "${temp_dir}/v2" >/dev/null

  echo "foreign protected file" > "${prefix}/bin/dosi-tracker"
  rm -f "$(meta_dir "${prefix}")/manifest.json"

  local failed="0"
  if bash "${INSTALLER}" rollback --prefix "${prefix}" >/dev/null 2>&1; then
    failed="1"
  fi
  local content
  content="$(cat "${prefix}/bin/dosi-tracker")"
  rm -rf "${temp_dir}"
  [ "${failed}" = "0" ] && [ "${content}" = "foreign protected file" ]
}

# 10. Rollback refuses a foreign-prefix or mismatched backup
test_rollback_refuses_foreign_prefix_or_mismatched_backup() {
  local temp_dir
  temp_dir="$(mktemp -d)"
  local prefix="${temp_dir}/opt/dosi"
  create_fake_executable "${temp_dir}/v1" "1.0.0"
  bash "${INSTALLER}" install --prefix "${prefix}" --source-bin "${temp_dir}/v1" >/dev/null

  local config_dir="${HOME}/.config/dosi-tracker"
  mkdir -p "${config_dir}"
  echo "precious_user_consent" > "${config_dir}/consent.json"
  echo "unrelated_sentinel_data" > "${prefix}/share/unrelated.txt"

  local backup_dir
  backup_dir="$(meta_dir "${prefix}")/backup"
  mkdir -p "${backup_dir}/files/bin"
  create_fake_executable "${backup_dir}/files/bin/dosi-tracker" "0.9.0"
  write_broken_manifest "/foreign/opt/dosi" "${backup_dir}/manifest.json.prev.tmp" trailing_garbage
  # A well-formed format 2 manifest, but bound to /foreign/opt/dosi.
  write_manifest_v2 "/foreign/opt/dosi" "${backup_dir}/manifest.json.prev" \
    "/foreign/opt/dosi/bin/dosi-tracker" "/foreign/opt/dosi/${META_SUBDIR}/manifest.json" \
    "/foreign/opt/dosi/${META_SUBDIR}/current_version"
  rm -f "${backup_dir}/manifest.json.prev.tmp"
  echo "0.9.0" > "${backup_dir}/current_version.prev"

  local cur_bin cur_manifest cur_version
  cur_bin="$("${prefix}/bin/dosi-tracker" --version)"
  cur_manifest="$(cat "$(meta_dir "${prefix}")/manifest.json")"
  cur_version="$(cat "$(meta_dir "${prefix}")/current_version")"

  local failed="0"
  if bash "${INSTALLER}" rollback --prefix "${prefix}" >/dev/null 2>&1; then
    failed="1"
  fi

  local ok=1
  [ "${cur_bin}" = "$("${prefix}/bin/dosi-tracker" --version)" ] || ok=0
  [ "${cur_manifest}" = "$(cat "$(meta_dir "${prefix}")/manifest.json")" ] || ok=0
  [ "${cur_version}" = "$(cat "$(meta_dir "${prefix}")/current_version")" ] || ok=0
  [ "$(cat "${config_dir}/consent.json")" = "precious_user_consent" ] || ok=0
  [ "$(cat "${prefix}/share/unrelated.txt")" = "unrelated_sentinel_data" ] || ok=0
  rm -rf "${temp_dir}" "${config_dir}"
  [ "${failed}" = "0" ] && [ "${ok}" = "1" ]
}

# 11. Rollback restores the complete managed state (binaries, desktop files, metadata)
test_rollback_restores_complete_managed_state() {
  local temp_dir
  temp_dir="$(mktemp -d)"
  local prefix="${temp_dir}/opt/dosi"
  create_fake_executable "${temp_dir}/v1" "0.1.0"
  create_fake_executable "${temp_dir}/v2" "0.2.0"
  create_fake_gui "${temp_dir}/gui2" "0.2.0"
  bash "${INSTALLER}" install --prefix "${prefix}" --source-bin "${temp_dir}/v1" >/dev/null
  # 0.2.0 adds the window, so its desktop entry appears too.
  bash "${INSTALLER}" update --prefix "${prefix}" --source-bin "${temp_dir}/v2" --source-gui "${temp_dir}/gui2" >/dev/null
  local had_gui=0
  [ -x "${prefix}/bin/dosi-tracker-gui" ] && [ -f "${prefix}/share/applications/${APP_ID}.desktop" ] && had_gui=1

  bash "${INSTALLER}" rollback --prefix "${prefix}" >/dev/null

  local ok=1
  [[ "$("${prefix}/bin/dosi-tracker" --version)" == *"0.1.0"* ]] || ok=0
  grep -q '"version": "0.1.0"' "$(meta_dir "${prefix}")/manifest.json" || ok=0
  [ "$(cat "$(meta_dir "${prefix}")/current_version")" = "0.1.0" ] || ok=0
  [ ! -e "${prefix}/bin/dosi-tracker-gui" ] || ok=0
  [ ! -e "${prefix}/share/applications/${APP_ID}.desktop" ] || ok=0
  # The restored install is fully managed again: a clean uninstall works.
  bash "${INSTALLER}" uninstall --prefix "${prefix}" >/dev/null || ok=0
  [ -z "$(find "${prefix}" -type f)" ] || ok=0
  rm -rf "${temp_dir}"
  [ "${had_gui}" = "1" ] && [ "${ok}" = "1" ]
}

# Shared by 12 and 13: an update with an injected failure restores 0.1.0 completely.
update_with_injected_failure_restores() {
  local inject="$1"
  local temp_dir
  temp_dir="$(mktemp -d)"
  local prefix="${temp_dir}/opt/dosi"
  create_fake_executable "${temp_dir}/v1" "0.1.0"
  create_fake_gui "${temp_dir}/gui1" "0.1.0"
  create_fake_executable "${temp_dir}/v2" "0.2.0"
  bash "${INSTALLER}" install --prefix "${prefix}" --source-bin "${temp_dir}/v1" --source-gui "${temp_dir}/gui1" >/dev/null
  local before_manifest before_files
  before_manifest="$(cat "$(meta_dir "${prefix}")/manifest.json")"
  before_files="$(cd "${prefix}" && find . -path "./${META_SUBDIR}/backup" -prune -o -type f -print | sort)"

  local failed="0"
  # 0.2.0 without the window: the update would remove it.
  if env "${inject}=1" bash "${INSTALLER}" update --prefix "${prefix}" --source-bin "${temp_dir}/v2" >/dev/null 2>&1; then
    failed="1"
  fi

  local ok=1
  [[ "$("${prefix}/bin/dosi-tracker" --version)" == *"0.1.0"* ]] || ok=0
  [ "$(cat "$(meta_dir "${prefix}")/current_version")" = "0.1.0" ] || ok=0
  [ "$(cat "$(meta_dir "${prefix}")/manifest.json")" = "${before_manifest}" ] || ok=0
  [ -x "${prefix}/bin/dosi-tracker-gui" ] || ok=0
  [ "$(cd "${prefix}" && find . -path "./${META_SUBDIR}/backup" -prune -o -type f -print | sort)" = "${before_files}" ] || ok=0
  rm -rf "${temp_dir}"
  [ "${failed}" = "0" ] && [ "${ok}" = "1" ]
}

# 12. Injected metadata publication failure
test_metadata_write_failure_fully_rolls_back() {
  update_with_injected_failure_restores DOSI_TEST_INJECT_FAIL_METADATA
}

# 13. Injected post-activation smoke failure
test_smoke_check_failure_fully_rolls_back() {
  update_with_injected_failure_restores DOSI_TEST_INJECT_FAIL_SMOKE
}

# 14. Uninstall refuses a corrupted manifest
test_uninstall_refuses_on_corrupted_manifest() {
  local ok=1 location
  for location in "${META_SUBDIR}" share/dosi-tracker; do
    local temp_dir
    temp_dir="$(mktemp -d)"
    local prefix="${temp_dir}/opt/dosi"
    mkdir -p "${prefix}/bin" "${prefix}/${location}"
    echo "precious user script" > "${prefix}/bin/dosi-tracker"
    echo "corrupted metadata" > "${prefix}/${location}/manifest.json"
    if bash "${INSTALLER}" uninstall --prefix "${prefix}" >/dev/null 2>&1; then
      ok=0
    fi
    [ "$(cat "${prefix}/bin/dosi-tracker")" = "precious user script" ] || ok=0
    rm -rf "${temp_dir}"
  done
  [ "${ok}" = "1" ]
}

# 15. Uninstall preserves user config and unrelated sentinels
test_uninstall_preserves_user_config_and_unrelated_sentinels() {
  local temp_dir
  temp_dir="$(mktemp -d)"
  local prefix="${temp_dir}/opt/dosi"
  create_fake_executable "${temp_dir}/fake_bin" "0.1.0"
  create_fake_gui "${temp_dir}/fake_gui" "0.1.0"
  bash "${INSTALLER}" install --prefix "${prefix}" --source-bin "${temp_dir}/fake_bin" --source-gui "${temp_dir}/fake_gui" >/dev/null

  echo "unrelated user tool" > "${prefix}/bin/other-tool"
  mkdir -p "${prefix}/share/other-app" "${prefix}/share/applications"
  echo "other app data" > "${prefix}/share/other-app/data.txt"
  echo "[Desktop Entry]" > "${prefix}/share/applications/other.desktop"

  local config="${HOME}/.config/dosi-tracker"
  mkdir -p "${config}"
  echo '{"screenshot":false}' > "${config}/consent.json"

  bash "${INSTALLER}" uninstall --prefix "${prefix}" >/dev/null

  local ok=1
  [ ! -e "${prefix}/bin/dosi-tracker" ] && [ ! -e "${prefix}/bin/dosi-tracker-gui" ] || ok=0
  [ ! -e "${prefix}/share/applications/${APP_ID}.desktop" ] || ok=0
  [ ! -e "$(meta_dir "${prefix}")" ] && [ ! -e "${prefix}/lib/dosi-tracker" ] || ok=0
  [ -f "${prefix}/bin/other-tool" ] && [ -f "${prefix}/share/other-app/data.txt" ] || ok=0
  [ -f "${prefix}/share/applications/other.desktop" ] || ok=0
  [ -f "${config}/consent.json" ] || ok=0
  rm -rf "${temp_dir}" "${config}"
  [ "${ok}" = "1" ]
}

# 16. Idempotent uninstall, and it creates nothing
test_idempotent_uninstall() {
  local temp_dir
  temp_dir="$(mktemp -d)"
  local prefix="${temp_dir}/opt/empty-prefix"
  bash "${INSTALLER}" uninstall --prefix "${prefix}" >/dev/null
  bash "${INSTALLER}" status --prefix "${prefix}" >/dev/null
  local created=0
  [ -e "${prefix}" ] && created=1
  rm -rf "${temp_dir}"
  [ "${created}" = "0" ]
}

# 17. Real compiled binaries from the checkout (cargo build) and the doctor smoke test
test_real_compiled_binary_install_smoke() {
  local temp_dir
  temp_dir="$(mktemp -d)"
  local prefix="${temp_dir}/opt/dosi-real"
  local ok=1
  bash "${INSTALLER}" install --prefix "${prefix}" >/dev/null || ok=0

  local bin_path="${prefix}/bin/dosi-tracker"
  [ -x "${bin_path}" ] && [ -x "${prefix}/bin/dosi-tracker-gui" ] || ok=0
  local doctor_json="" gui_version=""
  doctor_json="$("${bin_path}" doctor --json)" || ok=0
  gui_version="$("${prefix}/bin/dosi-tracker-gui" --version)" || ok=0
  grep -q 'dosi-tracker' "${prefix}/share/bash-completion/completions/dosi-tracker" || ok=0
  grep -q 'dosi-tracker' "${prefix}/share/zsh/site-functions/_dosi-tracker" || ok=0
  grep -q 'dosi-tracker' "${prefix}/share/fish/vendor_completions.d/dosi-tracker.fish" || ok=0
  grep -q '"source": "build"' "$(meta_dir "${prefix}")/manifest.json" || ok=0
  rm -rf "${temp_dir}"
  [ "${ok}" = "1" ] && [[ "${doctor_json}" == *"dosi-tracker-linux"* ]] \
    && [[ "${doctor_json}" == *"NOT_IMPLEMENTED"* ]] && [[ "${gui_version}" == "dosi-tracker-gui "* ]]
}

# 18. Unauthorized owned_files entries are refused before any mutation
test_uninstall_refuses_tampered_owned_files() {
  local temp_dir
  temp_dir="$(mktemp -d)"
  local prefix="${temp_dir}/opt/dosi"
  create_fake_executable "${temp_dir}/fake_bin" "0.1.0"
  local meta
  meta="$(meta_dir "${prefix}")"
  local outside="${temp_dir}/outside"
  mkdir -p "${outside}"
  echo "secret payload" > "${outside}/secret.txt"
  echo "foreign payload" > "${temp_dir}/foreign_sentinel.txt"

  local ok=1 entry
  for entry in \
    "${prefix}/unrelated_sentinel.txt" \
    "${prefix}/../../foreign_sentinel.txt" \
    "${prefix}/link_dir/secret.txt" \
    "${prefix}/bin/dosi-tracker\\n${prefix}/unrelated_sentinel.txt" \
    "  ${prefix}/bin/dosi-tracker  " \
    "${HOME}/.config/dosi-tracker/consent.json"; do
    rm -rf "${prefix}"
    bash "${INSTALLER}" install --prefix "${prefix}" --source-bin "${temp_dir}/fake_bin" >/dev/null
    echo "unrelated sentinel payload" > "${prefix}/unrelated_sentinel.txt"
    ln -s "${outside}" "${prefix}/link_dir"
    write_manifest_v2 "${prefix}" "${meta}/manifest.json" \
      "${prefix}/bin/dosi-tracker" "${meta}/manifest.json" "${meta}/current_version" "${entry}"
    if bash "${INSTALLER}" uninstall --prefix "${prefix}" >/dev/null 2>&1; then
      echo "uninstall accepted owned entry ${entry}" >&2
      ok=0
    fi
    [ -f "${prefix}/bin/dosi-tracker" ] && [ -f "${meta}/manifest.json" ] && [ -f "${meta}/current_version" ] || ok=0
    [ "$(cat "${prefix}/unrelated_sentinel.txt")" = "unrelated sentinel payload" ] || ok=0
  done
  [ "$(cat "${outside}/secret.txt")" = "secret payload" ] || ok=0
  [ "$(cat "${temp_dir}/foreign_sentinel.txt")" = "foreign payload" ] || ok=0
  rm -rf "${temp_dir}"
  [ "${ok}" = "1" ]
}

# 19. Update and rollback refuse unauthorized owned_files
test_update_and_rollback_refuse_unauthorized_owned_files() {
  local temp_dir
  temp_dir="$(mktemp -d)"
  local prefix="${temp_dir}/opt/dosi"
  create_fake_executable "${temp_dir}/v1" "0.1.0"
  create_fake_executable "${temp_dir}/v2" "0.2.0"
  bash "${INSTALLER}" install --prefix "${prefix}" --source-bin "${temp_dir}/v1" >/dev/null
  local meta
  meta="$(meta_dir "${prefix}")"
  local unrelated="${prefix}/unrelated_sentinel.txt"
  echo "preserve unrelated" > "${unrelated}"
  local good
  good="$(cat "${meta}/manifest.json")"

  write_manifest_v2 "${prefix}" "${meta}/manifest.json" \
    "${prefix}/bin/dosi-tracker" "${meta}/manifest.json" "${meta}/current_version" "${unrelated}"
  local update_failed="0"
  if bash "${INSTALLER}" update --prefix "${prefix}" --source-bin "${temp_dir}/v2" >/dev/null 2>&1; then
    update_failed="1"
  fi

  printf '%s\n' "${good}" > "${meta}/manifest.json"
  bash "${INSTALLER}" update --prefix "${prefix}" --source-bin "${temp_dir}/v2" >/dev/null

  write_manifest_v2 "${prefix}" "${meta}/backup/manifest.json.prev" \
    "${prefix}/bin/dosi-tracker" "${meta}/manifest.json" "${meta}/current_version" "${unrelated}"
  local rollback_failed="0"
  if bash "${INSTALLER}" rollback --prefix "${prefix}" >/dev/null 2>&1; then
    rollback_failed="1"
  fi

  local current_ver unrelated_ok=0
  current_ver="$(cat "${meta}/current_version")"
  [ "$(cat "${unrelated}")" = "preserve unrelated" ] && unrelated_ok=1
  rm -rf "${temp_dir}"
  [ "${update_failed}" = "0" ] && [ "${rollback_failed}" = "0" ] && [ "${current_ver}" = "0.2.0" ] && [ "${unrelated_ok}" = "1" ]
}

# 20. Inventory confinement: everything owned is in the prefix and in the inventory;
# nothing is written outside the prefix.
test_v2_inventory_confinement() {
  local temp_dir
  temp_dir="$(mktemp -d)"
  local prefix="${temp_dir}/opt/dosi"
  create_fake_executable "${temp_dir}/fake_bin" "0.1.0"
  create_fake_gui "${temp_dir}/fake_gui" "0.1.0"
  touch "${temp_dir}/.stamp"
  local home_before
  home_before="$(cd "${HOME}" && find . | sort)"

  bash "${INSTALLER}" install --prefix "${prefix}" --source-bin "${temp_dir}/fake_bin" --source-gui "${temp_dir}/fake_gui" >/dev/null

  local ok=1 meta
  meta="$(meta_dir "${prefix}")"
  local expected
  expected="$(cat <<EOF | sort
${prefix}/bin/dosi-tracker
${prefix}/bin/dosi-tracker-gui
${prefix}/share/applications/${APP_ID}.desktop
${prefix}/share/icons/hicolor/16x16/apps/${APP_ID}.png
${prefix}/share/icons/hicolor/22x22/apps/${APP_ID}.png
${prefix}/share/icons/hicolor/24x24/apps/${APP_ID}.png
${prefix}/share/icons/hicolor/32x32/apps/${APP_ID}.png
${prefix}/share/icons/hicolor/48x48/apps/${APP_ID}.png
${prefix}/share/icons/hicolor/64x64/apps/${APP_ID}.png
${prefix}/share/icons/hicolor/128x128/apps/${APP_ID}.png
${prefix}/share/icons/hicolor/256x256/apps/${APP_ID}.png
${prefix}/share/icons/hicolor/512x512/apps/${APP_ID}.png
${prefix}/share/icons/hicolor/scalable/apps/${APP_ID}.svg
${prefix}/share/bash-completion/completions/dosi-tracker
${prefix}/share/zsh/site-functions/_dosi-tracker
${prefix}/share/fish/vendor_completions.d/dosi-tracker.fish
${meta}/manifest.json
${meta}/current_version
${meta}/uninstall.sh
EOF
)"
  local owned
  owned="$(python3 -c 'import json,sys; print("\n".join(json.load(open(sys.argv[1]))["owned_files"]))' "${meta}/manifest.json" | sort)"
  if [ "${owned}" != "${expected}" ]; then
    diff <(printf '%s\n' "${expected}") <(printf '%s\n' "${owned}") >&2 || true
    ok=0
  fi
  # Exactly the owned files exist in the prefix (no staging left behind).
  [ "$(find "${prefix}" -type f | sort)" = "${expected}" ] || ok=0
  # Nothing outside the prefix: HOME untouched, no autostart, no user unit elsewhere.
  [ "$(cd "${HOME}" && find . | sort)" = "${home_before}" ] || ok=0
  [ -z "$(find "${temp_dir}" -newer "${temp_dir}/.stamp" -not -path "${prefix}*" \
    -not -path "${temp_dir}" -not -path "${temp_dir}/opt")" ] || ok=0
  # The desktop entry runs the installed programs by absolute path.
  grep -qx "Exec=${prefix}/bin/dosi-tracker-gui" "${prefix}/share/applications/${APP_ID}.desktop" || ok=0
  grep -qx "TryExec=${prefix}/bin/dosi-tracker-gui" "${prefix}/share/applications/${APP_ID}.desktop" || ok=0
  grep -qx "Exec=${prefix}/bin/dosi-tracker pause" "${prefix}/share/applications/${APP_ID}.desktop" || ok=0
  ! grep -q '@BINDIR@' "${prefix}/share/applications/${APP_ID}.desktop" || ok=0
  if command -v desktop-file-validate >/dev/null 2>&1; then
    desktop-file-validate "${prefix}/share/applications/${APP_ID}.desktop" || ok=0
  fi
  # Files are not group/world writable.
  [ -z "$(find "${prefix}" -type f -perm /022)" ] || ok=0
  rm -rf "${temp_dir}"
  [ "${ok}" = "1" ]
}

# 21. The user unit goes to ~/.local only, is never enabled, and no autostart entry is
# ever written; a prefix with a space still gets a valid quoted desktop entry.
test_v2_user_unit_and_no_autostart() {
  local temp_dir
  temp_dir="$(mktemp -d)"
  create_fake_executable "${temp_dir}/fake_bin" "0.1.0"
  create_fake_gui "${temp_dir}/fake_gui" "0.1.0"
  local ok=1
  (
    export HOME="${temp_dir}/home"
    mkdir -p "${HOME}" || exit 1
    bash "${INSTALLER}" install --source-bin "${temp_dir}/fake_bin" --source-gui "${temp_dir}/fake_gui" >/dev/null || exit 1
    bash "${INSTALLER}" update --source-bin "${temp_dir}/fake_bin" --source-gui "${temp_dir}/fake_gui" >/dev/null || exit 1
    unit="${HOME}/.local/share/systemd/user/dosi-tracker.service" || exit 1
    [ -f "${unit}" ] || exit 1
    grep -qx "ExecStart=${HOME}/.local/bin/dosi-tracker daemon --systemd" "${unit}" || exit 1
    if grep -q '^\[Install\]' "${unit}"; then exit 1; fi
    grep -q "\"${unit}\"" "${HOME}/.local/${META_SUBDIR}/manifest.json" || exit 1
    if command -v systemd-analyze >/dev/null 2>&1; then
      mkdir -p "${temp_dir}/run" && chmod 700 "${temp_dir}/run" || exit 1
      XDG_RUNTIME_DIR="${temp_dir}/run" systemd-analyze verify --user "${unit}" || exit 1
    fi
    # Nothing enabled, nothing started at login.
    [ ! -e "${HOME}/.config/autostart" ] || exit 1
    [ ! -e "${HOME}/.config/systemd" ] || exit 1
    [ -z "$(find "${HOME}/.local/share/systemd" \( -name '*.wants' -o -type l \) -print)" ] || exit 1
    bash "${INSTALLER}" uninstall >/dev/null || exit 1
    [ ! -e "${unit}" ] || exit 1
  ) || ok=0

  # Another prefix: no unit (service enable writes one), but the rest installs.
  local prefix="${temp_dir}/My Apps/dosi"
  bash "${INSTALLER}" install --prefix "${prefix}" --source-bin "${temp_dir}/fake_bin" --source-gui "${temp_dir}/fake_gui" >/dev/null || ok=0
  [ ! -e "${prefix}/share/systemd" ] || ok=0
  grep -qx "Exec=\"${prefix}/bin/dosi-tracker-gui\"" "${prefix}/share/applications/${APP_ID}.desktop" || ok=0
  grep -qx "TryExec=${prefix}/bin/dosi-tracker-gui" "${prefix}/share/applications/${APP_ID}.desktop" || ok=0
  if command -v desktop-file-validate >/dev/null 2>&1; then
    desktop-file-validate "${prefix}/share/applications/${APP_ID}.desktop" || ok=0
  fi
  bash "${INSTALLER}" uninstall --prefix "${prefix}" >/dev/null || ok=0
  [ -z "$(find "${prefix}" -type f)" ] || ok=0
  rm -rf "${temp_dir}"
  [ "${ok}" = "1" ]
}

# 22. A foundation (format 1) install is migrated; rollback returns to it.
test_v1_manifest_migration() {
  local temp_dir
  temp_dir="$(mktemp -d)"
  local prefix="${temp_dir}/opt/dosi"
  mkdir -p "${prefix}/bin"
  create_fake_executable "${prefix}/bin/dosi-tracker" "0.0.9"
  write_manifest_v1 "${prefix}" "0.0.9"
  mkdir -p "${prefix}/share/dosi-tracker/backup"
  create_fake_executable "${prefix}/share/dosi-tracker/backup/dosi-tracker.prev" "0.0.8"
  create_fake_executable "${temp_dir}/v1" "0.1.0"

  local ok=1
  bash "${INSTALLER}" install --prefix "${prefix}" --source-bin "${temp_dir}/v1" >/dev/null || ok=0
  local meta
  meta="$(meta_dir "${prefix}")"
  [[ "$("${prefix}/bin/dosi-tracker" --version)" == *"0.1.0"* ]] || ok=0
  grep -q '"format_version": 2' "${meta}/manifest.json" || ok=0
  grep -q '"previous_version": "0.0.9"' "${meta}/manifest.json" || ok=0
  # The foundation metadata (and its backup) are gone.
  [ ! -e "${prefix}/share/dosi-tracker" ] || ok=0

  bash "${INSTALLER}" rollback --prefix "${prefix}" >/dev/null || ok=0
  [[ "$("${prefix}/bin/dosi-tracker" --version)" == *"0.0.9"* ]] || ok=0
  [ "$(cat "${meta}/current_version")" = "0.0.9" ] || ok=0
  grep -q '"format_version": 2' "${meta}/manifest.json" || ok=0

  bash "${INSTALLER}" uninstall --prefix "${prefix}" >/dev/null || ok=0
  [ -z "$(find "${prefix}" -type f)" ] || ok=0

  # Plain uninstall of a format 1 install works as before.
  mkdir -p "${prefix}/bin"
  create_fake_executable "${prefix}/bin/dosi-tracker" "0.0.9"
  write_manifest_v1 "${prefix}" "0.0.9"
  bash "${INSTALLER}" uninstall --prefix "${prefix}" >/dev/null || ok=0
  [ -z "$(find "${prefix}" -type f)" ] && [ ! -e "${prefix}/share/dosi-tracker" ] || ok=0
  rm -rf "${temp_dir}"
  [ "${ok}" = "1" ]
}

# 23. Update stops a running agent, swaps and restarts it; a failed update brings the
# previous agent back.
test_update_restarts_running_agent() {
  local temp_dir
  temp_dir="$(mktemp -d)"
  local prefix="${temp_dir}/opt/dosi"
  local state="${temp_dir}/state"
  mkdir -p "${state}"
  create_fake_executable "${temp_dir}/v1" "0.1.0" "${state}"
  create_fake_executable "${temp_dir}/v2" "0.2.0" "${state}"
  create_fake_executable "${temp_dir}/v3" "0.3.0" "${state}"
  bash "${INSTALLER}" install --prefix "${prefix}" --source-bin "${temp_dir}/v1" >/dev/null
  touch "${state}/running"
  : > "${state}/calls"

  local ok=1
  bash "${INSTALLER}" update --prefix "${prefix}" --source-bin "${temp_dir}/v2" >/dev/null || ok=0
  grep -qx 'stop 0.1.0' "${state}/calls" || ok=0
  grep -qx 'start 0.2.0' "${state}/calls" || ok=0
  [ -e "${state}/running" ] || ok=0

  # 0.3.0 does not start: the update is rolled back and 0.2.0 runs again.
  : > "${state}/calls"
  touch "${state}/fail-start-0.3.0"
  if bash "${INSTALLER}" update --prefix "${prefix}" --source-bin "${temp_dir}/v3" >/dev/null 2>&1; then
    ok=0
  fi
  grep -qx 'stop 0.2.0' "${state}/calls" || ok=0
  grep -qx 'start 0.3.0' "${state}/calls" || ok=0
  [ "$(tail -n1 "${state}/calls")" = "start 0.2.0" ] || ok=0
  [[ "$("${prefix}/bin/dosi-tracker" --version)" == *"0.2.0"* ]] || ok=0
  [ "$(cat "$(meta_dir "${prefix}")/current_version")" = "0.2.0" ] || ok=0

  # An agent that is not running is not started by an update.
  rm -f "${state}/running" "${state}/fail-start-0.3.0"
  : > "${state}/calls"
  bash "${INSTALLER}" update --prefix "${prefix}" --source-bin "${temp_dir}/v3" >/dev/null || ok=0
  ! grep -q '^start' "${state}/calls" || ok=0
  rm -rf "${temp_dir}"
  [ "${ok}" = "1" ]
}

# 24. From a tarball layout (bin/ next to install.sh), and the installed uninstaller.
test_tarball_mode_and_installed_uninstaller() {
  local temp_dir
  temp_dir="$(mktemp -d)"
  local tree="${temp_dir}/dosi-tracker-0.1.0"
  mkdir -p "${tree}/bin"
  cp "${INSTALLER}" "${tree}/install.sh"
  cp "${LINUX_CRATE_DIR}/scripts/uninstall.sh" "${tree}/uninstall.sh"
  cp -r "${LINUX_CRATE_DIR}/packaging" "${tree}/packaging"
  echo "0.1.0" > "${tree}/VERSION"
  create_fake_executable "${tree}/bin/dosi-tracker" "0.1.0"
  create_fake_gui "${tree}/bin/dosi-tracker-gui" "0.1.0"
  local prefix="${temp_dir}/prefix"

  local ok=1
  [ "$(bash "${tree}/install.sh" --version)" = "dosi-tracker installer version 0.1.0" ] || ok=0
  bash "${tree}/install.sh" --prefix "${prefix}" >/dev/null || ok=0
  [ -x "${prefix}/bin/dosi-tracker" ] && [ -x "${prefix}/bin/dosi-tracker-gui" ] || ok=0
  grep -q '"source": "tarball"' "$(meta_dir "${prefix}")/manifest.json" || ok=0
  [ -f "${prefix}/share/icons/hicolor/scalable/apps/${APP_ID}.svg" ] || ok=0

  # The installed copy uninstalls its own prefix by default, from anywhere.
  local uninstaller
  uninstaller="$(meta_dir "${prefix}")/uninstall.sh"
  (cd / && bash "${uninstaller}" --yes >/dev/null) || ok=0
  [ -z "$(find "${prefix}" -type f)" ] || ok=0
  [ ! -e "${prefix}/lib/dosi-tracker" ] || ok=0

  # The tarball's uninstall.sh wrapper works too.
  bash "${tree}/install.sh" install --prefix "${prefix}" >/dev/null || ok=0
  bash "${tree}/uninstall.sh" --prefix "${prefix}" >/dev/null || ok=0
  [ -z "$(find "${prefix}" -type f)" ] || ok=0
  rm -rf "${temp_dir}"
  [ "${ok}" = "1" ]
}

# 25. Root is refused before anything changes
test_root_refused() {
  local temp_dir
  temp_dir="$(mktemp -d)"
  local prefix="${temp_dir}/opt/dosi"
  create_fake_executable "${temp_dir}/fake_bin" "0.1.0"
  local ok=1 out
  if out="$(USER=root bash "${INSTALLER}" install --prefix "${prefix}" --source-bin "${temp_dir}/fake_bin" 2>&1)"; then
    ok=0
  fi
  [[ "${out}" == *"Refusing to run as root"* ]] || ok=0
  if USER=root bash "${INSTALLER}" uninstall --prefix "${prefix}" >/dev/null 2>&1; then
    ok=0
  fi
  # With a user namespace the effective uid is 0 as well (skipped where unavailable).
  if unshare -r true >/dev/null 2>&1; then
    if unshare -r bash "${INSTALLER}" install --prefix "${prefix}" --source-bin "${temp_dir}/fake_bin" >/dev/null 2>&1; then
      ok=0
    fi
  fi
  [ ! -e "${prefix}" ] || ok=0
  rm -rf "${temp_dir}"
  [ "${ok}" = "1" ]
}

# 26. --server/--dashboard go through `dosi-tracker config set`: an invalid URL exits 2
# before any change, valid ones are saved in the user's configuration.
test_server_option_validation() {
  ensure_real_binaries || return 1
  local temp_dir
  temp_dir="$(mktemp -d)"
  local prefix="${temp_dir}/opt/dosi"
  local ok=1 code=0
  (
    export HOME="${temp_dir}/home"
    mkdir -p "${HOME}" || exit 1
    bash "${INSTALLER}" install --prefix "${prefix}" --source-bin "${REAL_BIN}" \
      --server 'https://tracker.example.com/?tenant=x' >/dev/null 2>&1
  ) || code=$?
  [ "${code}" = "2" ] || { echo "invalid --server exited ${code}" >&2; ok=0; }
  code=0
  (
    export HOME="${temp_dir}/home"
    bash "${INSTALLER}" install --prefix "${prefix}" --source-bin "${REAL_BIN}" \
      --server tracker.example.com --dashboard 'ftp://app.example.com' >/dev/null 2>&1
  ) || code=$?
  [ "${code}" = "2" ] || { echo "invalid --dashboard exited ${code}" >&2; ok=0; }
  [ ! -e "${prefix}" ] && [ ! -e "${temp_dir}/home/.config" ] || ok=0

  (
    export HOME="${temp_dir}/home"
    bash "${INSTALLER}" install --prefix "${prefix}" --source-bin "${REAL_BIN}" --source-gui "${REAL_GUI}" \
      --server tracker.example.com --dashboard https://app.example.com >/dev/null || exit 1
    [ "$("${prefix}/bin/dosi-tracker" config get api_base_url)" = "https://tracker.example.com" ] || exit 1
    [ "$("${prefix}/bin/dosi-tracker" config get dashboard_url)" = "https://app.example.com" ] || exit 1
    [ "$(stat -c %a "${HOME}/.config/dosi-tracker/config.toml")" = "600" ] || exit 1
  ) || ok=0
  rm -rf "${temp_dir}"
  [ "${ok}" = "1" ]
}

# 27. Plain uninstall keeps config, consent and data; --purge deletes them with
# `dosi-tracker reset` and needs --yes without a terminal.
test_purge_and_plain_uninstall() {
  ensure_real_binaries || return 1
  local temp_dir
  temp_dir="$(mktemp -d)"
  local prefix="${temp_dir}/opt/dosi"
  local ok=1
  (
    export HOME="${temp_dir}/home"
    mkdir -p "${HOME}/.config/dosi-tracker" "${HOME}/.local/share/dosi-tracker" "${HOME}/.local/state/dosi-tracker" || exit 1
    chmod -R go-rwx "${HOME}" || exit 1
    echo '{"granted":[]}' > "${HOME}/.config/dosi-tracker/consent.json" || exit 1
    echo 'sentinel' > "${HOME}/.local/share/dosi-tracker/sentinel" || exit 1
    echo 'sentinel' > "${HOME}/.local/state/dosi-tracker/sentinel" || exit 1

    bash "${INSTALLER}" install --prefix "${prefix}" --source-bin "${REAL_BIN}" >/dev/null || exit 1
    bash "${INSTALLER}" uninstall --prefix "${prefix}" >/dev/null || exit 1
    [ ! -e "${prefix}/bin/dosi-tracker" ] || exit 1
    [ -f "${HOME}/.config/dosi-tracker/consent.json" ] || exit 1
    [ -f "${HOME}/.local/share/dosi-tracker/sentinel" ] || exit 1
    [ -f "${HOME}/.local/state/dosi-tracker/sentinel" ] || exit 1

    bash "${INSTALLER}" install --prefix "${prefix}" --source-bin "${REAL_BIN}" >/dev/null || exit 1
    # No terminal and no --yes: refused with exit 2, nothing removed.
    code=0
    bash "${INSTALLER}" uninstall --prefix "${prefix}" --purge </dev/null >/dev/null 2>&1 || code=$?
    [ "${code}" = "2" ] || exit 1
    [ -x "${prefix}/bin/dosi-tracker" ] && [ -f "${HOME}/.local/share/dosi-tracker/sentinel" ] || exit 1

    bash "${INSTALLER}" uninstall --prefix "${prefix}" --purge --yes </dev/null >/dev/null || exit 1
    [ ! -e "${prefix}/bin/dosi-tracker" ] || exit 1
    [ ! -e "${HOME}/.config/dosi-tracker" ] || exit 1
    [ ! -e "${HOME}/.local/share/dosi-tracker" ] || exit 1
    [ ! -e "${HOME}/.local/state/dosi-tracker" ] || exit 1
  ) || ok=0
  rm -rf "${temp_dir}"
  [ "${ok}" = "1" ]
}

# 28. `dosi-tracker uninstall` finds and runs the installed uninstaller.
test_cli_uninstall_runs_installed_uninstaller() {
  ensure_real_binaries || return 1
  local temp_dir
  temp_dir="$(mktemp -d)"
  local prefix="${temp_dir}/opt/dosi"
  local ok=1
  (
    export HOME="${temp_dir}/home"
    mkdir -p "${HOME}/.config/dosi-tracker" || exit 1
    chmod -R go-rwx "${HOME}" || exit 1
    echo '{"granted":[]}' > "${HOME}/.config/dosi-tracker/consent.json" || exit 1
    bash "${INSTALLER}" install --prefix "${prefix}" --source-bin "${REAL_BIN}" --source-gui "${REAL_GUI}" >/dev/null || exit 1
    "${prefix}/bin/dosi-tracker" --json uninstall --yes | python3 -c 'import json,sys; d=json.load(sys.stdin); sys.exit(0 if d["ok"] and d["result"]["uninstalled"] else 1)' || exit 1
    [ -z "$(find "${prefix}" -type f)" ] || exit 1
    [ -f "${HOME}/.config/dosi-tracker/consent.json" ] || exit 1
  ) || ok=0
  rm -rf "${temp_dir}"
  [ "${ok}" = "1" ]
}

echo "=== Running Dosi-Tracker Linux Installer Lifecycle Tests ==="
run_test "Fresh Install to Isolated Prefix" test_fresh_install_to_isolated_temp_prefix
run_test "Foreign File Conflict Refusal" test_foreign_file_conflict_refusal
run_test "Malformed JSON Manifest Refusal" test_malformed_manifest_refusal
run_test "Duplicate Keys Manifest Refusal" test_duplicate_keys_manifest_refusal
run_test "Invalid owned_files Structure Refusal" test_invalid_owned_files_refusal
run_test "Tampered owned_files Entries Refusal" test_uninstall_refuses_tampered_owned_files
run_test "Update & Rollback Refuse Unauthorized owned_files" test_update_and_rollback_refuse_unauthorized_owned_files
run_test "Mismatched Prefix Manifest Refusal" test_mismatched_prefix_manifest_refusal
run_test "Sibling Prefix & Dot-Dot Traversal Refusal" test_sibling_prefix_and_dot_dot_escape_refusal
run_test "Ancestor & Explicit Symlink Escape Refusal" test_ancestor_and_explicit_symlink_escape_refusal
run_test "Rollback Refuses Foreign Destination" test_rollback_refuses_to_overwrite_unmanaged_foreign_destination
run_test "Rollback Refuses Foreign-Prefix or Mismatched Backup" test_rollback_refuses_foreign_prefix_or_mismatched_backup
run_test "Rollback Restores Complete Managed State" test_rollback_restores_complete_managed_state
run_test "Metadata Failure Triggers Complete Rollback" test_metadata_write_failure_fully_rolls_back
run_test "Smoke Failure Triggers Complete Rollback" test_smoke_check_failure_fully_rolls_back
run_test "Uninstall Refuses on Corrupt Manifest" test_uninstall_refuses_on_corrupted_manifest
run_test "Uninstall Preserves Config & Sentinels" test_uninstall_preserves_user_config_and_unrelated_sentinels
run_test "Idempotent Uninstall" test_idempotent_uninstall
run_test "v2 Inventory Confinement" test_v2_inventory_confinement
run_test "v2 User Unit Never Enabled, No Autostart" test_v2_user_unit_and_no_autostart
run_test "v1 Manifest Migration" test_v1_manifest_migration
run_test "Update Restarts a Running Agent" test_update_restarts_running_agent
run_test "Tarball Mode & Installed Uninstaller" test_tarball_mode_and_installed_uninstaller
run_test "Root Refused" test_root_refused
run_test "Real Compiled Binary Smoke & Doctor" test_real_compiled_binary_install_smoke
run_test "--server/--dashboard Validation" test_server_option_validation
run_test "Purge vs Plain Uninstall" test_purge_and_plain_uninstall
run_test "CLI uninstall Runs the Installed Uninstaller" test_cli_uninstall_runs_installed_uninstaller

echo "=== Test Summary ==="
echo "Passed: ${PASSED_TESTS}"
echo "Failed: ${FAILED_TESTS}"

if [ "${FAILED_TESTS}" -gt 0 ]; then
  exit 1
fi
exit 0
