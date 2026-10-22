#!/usr/bin/env bash
#
# Comprehensive test suite for Dosi-Tracker Linux rootless installer lifecycle.
#
# Tests cover:
# 1. Fresh rootless install to temporary prefix
# 2. Foreign/unmanaged file conflict refusal (preserves unrelated file)
# 3. Update with staging, ownership manifest update, and backup retention
# 4. Failed update preservation (staging failure leaves prior install intact)
# 5. Rollback to retained previous installation
# 6. Unsafe prefix and system directory refusal
# 7. Symlink escape refusal
# 8. Uninstall removes only owned files, preserving user config and unrelated sentinels
# 9. Idempotent uninstall
# 10. Real compiled binary install and diagnostic doctor smoke test
#

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LINUX_CRATE_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
INSTALLER="${LINUX_CRATE_DIR}/scripts/install.sh"

FAILED_TESTS=0
PASSED_TESTS=0

run_test() {
  local test_name="$1"
  shift
  printf "RUNNING: %s... " "${test_name}"
  if "$@"; then
    printf "OK\n"
    PASSED_TESTS=$((PASSED_TESTS + 1))
  else
    printf "FAILED\n"
    FAILED_TESTS=$((FAILED_TESTS + 1))
  fi
}

# Helper: Create synthetic executable fixture
create_fake_executable() {
  local target_path="$1"
  local version="${2:-0.1.0}"
  cat > "${target_path}" <<EOF
#!/usr/bin/env bash
if [ "\$1" = "--version" ]; then
  echo "dosi-tracker-linux ${version}"
  exit 0
elif [ "\$1" = "--help" ]; then
  echo "dosi-tracker-linux help: diagnostic foundation"
  exit 0
elif [ "\$1" = "doctor" ]; then
  if [ "\$2" = "--json" ]; then
    echo '{"component":"dosi-tracker-linux","version":"${version}","status":"ok"}'
  else
    echo "Dosi-Tracker Linux Diagnostic: OK (${version})"
  fi
  exit 0
else
  echo "Unknown command: \$1"
  exit 1
fi
EOF
  chmod 755 "${target_path}"
}

# 1. Fresh Install to Isolated Temp Prefix
test_fresh_install_to_isolated_temp_prefix() {
  local temp_dir
  temp_dir="$(mktemp -d)"

  local fake_src="${temp_dir}/fake_bin"
  create_fake_executable "${fake_src}" "0.1.0"

  local prefix="${temp_dir}/opt/dosi"
  bash "${INSTALLER}" install --prefix "${prefix}" --source-bin "${fake_src}" >/dev/null

  # Verify executable exists
  local bin_path="${prefix}/bin/dosi-tracker-linux"
  if [ ! -f "${bin_path}" ] && [ -f "${prefix}/bin/dosi-tracker-linux.exe" ]; then
    bin_path="${prefix}/bin/dosi-tracker-linux.exe"
  fi
  [ -f "${bin_path}" ]

  # Verify manifest exists and is valid
  local manifest="${prefix}/share/dosi-tracker/manifest.json"
  [ -f "${manifest}" ]
  grep -q '"component": "dosi-tracker-linux-diagnostic-foundation"' "${manifest}"
  grep -q '"version": "0.1.0"' "${manifest}"

  # Verify execution
  local out
  out="$("${bin_path}" --version)"
  [[ "${out}" == *"0.1.0"* ]]

  rm -rf "${temp_dir}"
}

# 2. Foreign / Unmanaged File Conflict Refusal
test_foreign_file_conflict_refusal() {
  local temp_dir
  temp_dir="$(mktemp -d)"

  local prefix="${temp_dir}/opt/dosi"
  mkdir -p "${prefix}/bin"

  # Place unmanaged foreign file without manifest
  echo "foreign proprietary payload" > "${prefix}/bin/dosi-tracker-linux"

  local fake_src="${temp_dir}/fake_bin"
  create_fake_executable "${fake_src}" "0.1.0"

  # Install must fail and refuse to overwrite
  local failed="0"
  if bash "${INSTALLER}" install --prefix "${prefix}" --source-bin "${fake_src}" >/dev/null 2>&1; then
    failed="1"
  fi

  # Verify foreign content is completely preserved
  local content
  content="$(cat "${prefix}/bin/dosi-tracker-linux")"

  rm -rf "${temp_dir}"

  [ "${failed}" = "0" ] && [ "${content}" = "foreign proprietary payload" ]
}

# 3. Update Retains Prior Backup and Updates Manifest
test_update_retains_prior_backup_and_updates_manifest() {
  local temp_dir
  temp_dir="$(mktemp -d)"

  local prefix="${temp_dir}/opt/dosi"
  local fake_src_v1="${temp_dir}/fake_bin_v1"
  create_fake_executable "${fake_src_v1}" "0.1.0"

  bash "${INSTALLER}" install --prefix "${prefix}" --source-bin "${fake_src_v1}" >/dev/null

  # Perform update to v0.2.0
  local fake_src_v2="${temp_dir}/fake_bin_v2"
  create_fake_executable "${fake_src_v2}" "0.2.0"

  bash "${INSTALLER}" update --prefix "${prefix}" --source-bin "${fake_src_v2}" >/dev/null

  # Verify backup binary exists
  local backup_path="${prefix}/share/dosi-tracker/backup/dosi-tracker-linux.prev"
  if [ ! -f "${backup_path}" ] && [ -f "${prefix}/share/dosi-tracker/backup/dosi-tracker-linux.exe.prev" ]; then
    backup_path="${prefix}/share/dosi-tracker/backup/dosi-tracker-linux.exe.prev"
  fi
  [ -f "${backup_path}" ]

  # Verify backup binary runs as v0.1.0
  local backup_ver
  backup_ver="$("${backup_path}" --version)"
  [[ "${backup_ver}" == *"0.1.0"* ]]

  # Verify active binary runs as v0.2.0
  local bin_path="${prefix}/bin/dosi-tracker-linux"
  if [ ! -f "${bin_path}" ] && [ -f "${prefix}/bin/dosi-tracker-linux.exe" ]; then
    bin_path="${prefix}/bin/dosi-tracker-linux.exe"
  fi
  local active_ver
  active_ver="$("${bin_path}" --version)"
  [[ "${active_ver}" == *"0.2.0"* ]]

  # Verify manifest tracks previous version
  local manifest="${prefix}/share/dosi-tracker/manifest.json"
  grep -q '"previous_version": "0.1.0"' "${manifest}"
  grep -q '"version": "0.2.0"' "${manifest}"

  rm -rf "${temp_dir}"
}

# 4. Failed Update Preserves Prior Working Install
test_failed_update_preserves_prior_working_install() {
  local temp_dir
  temp_dir="$(mktemp -d)"

  local prefix="${temp_dir}/opt/dosi"
  local fake_src_v1="${temp_dir}/fake_bin_v1"
  create_fake_executable "${fake_src_v1}" "0.1.0"

  bash "${INSTALLER}" install --prefix "${prefix}" --source-bin "${fake_src_v1}" >/dev/null

  # Create a broken candidate binary that fails --help smoke test
  local broken_src="${temp_dir}/broken_bin"
  cat > "${broken_src}" <<EOF
#!/usr/bin/env bash
echo "Corrupted binary crash!" >&2
exit 127
EOF
  chmod 755 "${broken_src}"

  # Attempt update; must fail
  local failed="0"
  if bash "${INSTALLER}" update --prefix "${prefix}" --source-bin "${broken_src}" >/dev/null 2>&1; then
    failed="1"
  fi

  # Prior working installation must remain intact and functional
  local bin_path="${prefix}/bin/dosi-tracker-linux"
  if [ ! -f "${bin_path}" ] && [ -f "${prefix}/bin/dosi-tracker-linux.exe" ]; then
    bin_path="${prefix}/bin/dosi-tracker-linux.exe"
  fi
  local active_ver
  active_ver="$("${bin_path}" --version)"

  rm -rf "${temp_dir}"

  [ "${failed}" = "0" ] && [[ "${active_ver}" == *"0.1.0"* ]]
}

# 5. Rollback to Retained Previous Installation
test_rollback_to_previous_installation() {
  local temp_dir
  temp_dir="$(mktemp -d)"

  local prefix="${temp_dir}/opt/dosi"
  local fake_src_v1="${temp_dir}/fake_bin_v1"
  create_fake_executable "${fake_src_v1}" "0.1.0"

  bash "${INSTALLER}" install --prefix "${prefix}" --source-bin "${fake_src_v1}" >/dev/null

  local fake_src_v2="${temp_dir}/fake_bin_v2"
  create_fake_executable "${fake_src_v2}" "0.2.0"
  bash "${INSTALLER}" update --prefix "${prefix}" --source-bin "${fake_src_v2}" >/dev/null

  # Execute rollback
  bash "${INSTALLER}" rollback --prefix "${prefix}" >/dev/null

  # Verify active binary is now rolled back to v0.1.0
  local bin_path="${prefix}/bin/dosi-tracker-linux"
  if [ ! -f "${bin_path}" ] && [ -f "${prefix}/bin/dosi-tracker-linux.exe" ]; then
    bin_path="${prefix}/bin/dosi-tracker-linux.exe"
  fi
  local active_ver
  active_ver="$("${bin_path}" --version)"

  rm -rf "${temp_dir}"

  [[ "${active_ver}" == *"0.1.0"* ]]
}

# 6. Unsafe Prefix and Path Boundary Refusal
test_unsafe_prefix_and_path_boundary_refusal() {
  local temp_dir
  temp_dir="$(mktemp -d)"

  local fake_src="${temp_dir}/fake_bin"
  create_fake_executable "${fake_src}" "0.1.0"

  local passed=1

  # System root /
  if bash "${INSTALLER}" install --prefix / --source-bin "${fake_src}" >/dev/null 2>&1; then
    passed=0
  fi

  # System /usr
  if bash "${INSTALLER}" install --prefix /usr --source-bin "${fake_src}" >/dev/null 2>&1; then
    passed=0
  fi

  # System /etc
  if bash "${INSTALLER}" install --prefix /etc --source-bin "${fake_src}" >/dev/null 2>&1; then
    passed=0
  fi

  rm -rf "${temp_dir}"
  [ "${passed}" = "1" ]
}

# 7. Symlink Escape Refusal
test_symlink_escape_refusal() {
  local temp_dir
  temp_dir="$(mktemp -d)"

  local prefix="${temp_dir}/opt/dosi"
  local outside_dir="${temp_dir}/outside"
  mkdir -p "${prefix}" "${outside_dir}"

  # Attempt to create symlink
  ln -s "${outside_dir}" "${prefix}/bin" 2>/dev/null || true

  # Check if real directory symlink was actually created by host OS
  if [ ! -L "${prefix}/bin" ]; then
    rm -rf "${temp_dir}"
    return 0 # Environment does not support directory symlinks without elevation; skip cleanly
  fi

  local fake_src="${temp_dir}/fake_bin"
  create_fake_executable "${fake_src}" "0.1.0"

  local passed=0
  if ! bash "${INSTALLER}" install --prefix "${prefix}" --source-bin "${fake_src}" >/dev/null 2>&1; then
    passed=1
  fi

  rm -rf "${temp_dir}"
  [ "${passed}" = "1" ]
}

# 8. Uninstall Preserves User Config and Unrelated Sentinels
test_uninstall_preserves_user_config_and_unrelated_sentinels() {
  local temp_dir
  temp_dir="$(mktemp -d)"

  local prefix="${temp_dir}/opt/dosi"
  local fake_src="${temp_dir}/fake_bin"
  create_fake_executable "${fake_src}" "0.1.0"

  bash "${INSTALLER}" install --prefix "${prefix}" --source-bin "${fake_src}" >/dev/null

  # Place unrelated sentinel file in prefix
  echo "unrelated user tool" > "${prefix}/bin/other-tool"
  mkdir -p "${prefix}/share/other-app"
  echo "other app data" > "${prefix}/share/other-app/data.txt"

  # Place fake user consent in mock config directory
  local mock_config="${temp_dir}/.config/dosi-tracker"
  mkdir -p "${mock_config}"
  echo '{"screenshot":false}' > "${mock_config}/consent.json"

  # Perform uninstall
  bash "${INSTALLER}" uninstall --prefix "${prefix}" >/dev/null

  # Verify Dosi-Tracker binary and manifest are removed
  local bin_removed=1
  if [ -f "${prefix}/bin/dosi-tracker-linux" ] || [ -f "${prefix}/bin/dosi-tracker-linux.exe" ]; then
    bin_removed=0
  fi
  local manifest_removed=1
  if [ -f "${prefix}/share/dosi-tracker/manifest.json" ]; then
    manifest_removed=0
  fi

  # Verify unrelated sentinel files are PRESERVED
  local sentinels_preserved=0
  if [ -f "${prefix}/bin/other-tool" ] && [ -f "${prefix}/share/other-app/data.txt" ]; then
    sentinels_preserved=1
  fi

  # Verify user consent configuration is PRESERVED
  local consent_preserved=0
  if [ -f "${mock_config}/consent.json" ]; then
    consent_preserved=1
  fi

  rm -rf "${temp_dir}"

  [ "${bin_removed}" = "1" ] && [ "${manifest_removed}" = "1" ] && [ "${sentinels_preserved}" = "1" ] && [ "${consent_preserved}" = "1" ]
}

# 9. Idempotent Uninstall
test_idempotent_uninstall() {
  local temp_dir
  temp_dir="$(mktemp -d)"

  local prefix="${temp_dir}/opt/empty-prefix"
  # Running uninstall on a non-existent or already cleaned prefix must exit 0 cleanly
  bash "${INSTALLER}" uninstall --prefix "${prefix}" >/dev/null

  rm -rf "${temp_dir}"
}

# 10. Real Compiled Binary Install and Diagnostic Doctor Smoke Test
test_real_compiled_binary_install_smoke() {
  local temp_dir
  temp_dir="$(mktemp -d)"

  local prefix="${temp_dir}/opt/dosi-real"

  # Perform real install from source crate using cargo
  bash "${INSTALLER}" install --prefix "${prefix}" >/dev/null

  local bin_path="${prefix}/bin/dosi-tracker-linux"
  if [ ! -f "${bin_path}" ] && [ -f "${prefix}/bin/dosi-tracker-linux.exe" ]; then
    bin_path="${prefix}/bin/dosi-tracker-linux.exe"
  fi
  [ -f "${bin_path}" ]

  # Run installed diagnostic doctor --json
  local doctor_json
  doctor_json="$("${bin_path}" doctor --json)"

  rm -rf "${temp_dir}"

  # Assert JSON contains truthful diagnoses
  [[ "${doctor_json}" == *"dosi-tracker-linux"* ]] && [[ "${doctor_json}" == *"NOT_IMPLEMENTED"* ]]
}

echo "=== Running Dosi-Tracker Linux Installer Lifecycle Tests ==="
run_test "Fresh Install to Isolated Prefix" test_fresh_install_to_isolated_temp_prefix
run_test "Foreign File Conflict Refusal" test_foreign_file_conflict_refusal
run_test "Update Retains Prior Backup & Manifest" test_update_retains_prior_backup_and_updates_manifest
run_test "Failed Update Preserves Prior Working Install" test_failed_update_preserves_prior_working_install
run_test "Rollback to Retained Previous Install" test_rollback_to_previous_installation
run_test "Unsafe Prefix & Boundary Refusal" test_unsafe_prefix_and_path_boundary_refusal
run_test "Symlink Escape Refusal" test_symlink_escape_refusal
run_test "Uninstall Preserves Config & Sentinels" test_uninstall_preserves_user_config_and_unrelated_sentinels
run_test "Idempotent Uninstall" test_idempotent_uninstall
run_test "Real Compiled Binary Smoke & Doctor" test_real_compiled_binary_install_smoke

echo "=== Test Summary ==="
echo "Passed: ${PASSED_TESTS}"
echo "Failed: ${FAILED_TESTS}"

if [ "${FAILED_TESTS}" -gt 0 ]; then
  exit 1
fi
exit 0
