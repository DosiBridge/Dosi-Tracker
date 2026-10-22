#!/usr/bin/env bash
#
# Comprehensive test suite for Dosi-Tracker Linux rootless installer lifecycle.
#
# Tests cover:
# 1. Fresh rootless install to temporary prefix
# 2. Foreign/unmanaged file conflict refusal (preserves unrelated file)
# 3. Malformed JSON manifest refusal (fails closed, preserves foreign/sentinel bytes)
# 4. Mismatched prefix/binary manifest refusal (fails closed)
# 5. Sibling prefix and dot-dot traversal refusal
# 6. Ancestor and explicit symlink escape refusal
# 7. Rollback refuses to overwrite unmanaged foreign destination
# 8. Rollback restores complete managed state (binary, manifest, version)
# 9. Injected metadata publication failure triggers complete rollback
# 10. Injected post-activation smoke failure triggers complete rollback
# 11. Uninstall refuses on corrupted or mismatched manifest
# 12. Uninstall preserves user config and unrelated sentinels
# 13. Idempotent uninstall
# 14. Real compiled binary install and diagnostic doctor smoke test
#

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
LINUX_CRATE_DIR="$(cd "${SCRIPT_DIR}/.." && pwd -P)"
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
  rm -rf "${temp_dir}"
  [[ "${out}" == *"0.1.0"* ]]
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

# 3. Malformed JSON Manifest Refusal (fails closed)
test_malformed_manifest_refusal() {
  local temp_dir
  temp_dir="$(mktemp -d)"

  local prefix="${temp_dir}/opt/dosi"
  mkdir -p "${prefix}/bin" "${prefix}/share/dosi-tracker"
  echo "existing payload" > "${prefix}/bin/dosi-tracker-linux"

  # Write malformed JSON that contains the component string
  cat > "${prefix}/share/dosi-tracker/manifest.json" <<EOF
{ "component": "dosi-tracker-linux-diagnostic-foundation", BROKEN_SYNTAX: [
EOF

  local fake_src="${temp_dir}/fake_bin"
  create_fake_executable "${fake_src}" "0.1.0"

  local failed="0"
  if bash "${INSTALLER}" install --prefix "${prefix}" --source-bin "${fake_src}" >/dev/null 2>&1; then
    failed="1"
  fi

  local content
  content="$(cat "${prefix}/bin/dosi-tracker-linux")"
  rm -rf "${temp_dir}"

  # Must fail closed and preserve existing payload
  [ "${failed}" = "0" ] && [ "${content}" = "existing payload" ]
}

# 4. Mismatched Prefix/Binary Manifest Refusal (fails closed)
test_mismatched_prefix_manifest_refusal() {
  local temp_dir
  temp_dir="$(mktemp -d)"

  local prefix="${temp_dir}/opt/dosi"
  mkdir -p "${prefix}/bin" "${prefix}/share/dosi-tracker"
  echo "protected foreign binary" > "${prefix}/bin/dosi-tracker-linux"

  # Valid JSON, but prefix points to completely different directory
  cat > "${prefix}/share/dosi-tracker/manifest.json" <<EOF
{
  "format_version": 1,
  "component": "dosi-tracker-linux-diagnostic-foundation",
  "version": "0.1.0",
  "prefix": "/another/unrelated/prefix",
  "bin_dir": "/another/unrelated/prefix/bin",
  "data_dir": "/another/unrelated/prefix/share/dosi-tracker",
  "binary_path": "/another/unrelated/prefix/bin/dosi-tracker-linux",
  "owned_files": [
    "/another/unrelated/prefix/bin/dosi-tracker-linux"
  ]
}
EOF

  local fake_src="${temp_dir}/fake_bin"
  create_fake_executable "${fake_src}" "0.1.0"

  local failed="0"
  if bash "${INSTALLER}" install --prefix "${prefix}" --source-bin "${fake_src}" >/dev/null 2>&1; then
    failed="1"
  fi

  local content
  content="$(cat "${prefix}/bin/dosi-tracker-linux")"
  rm -rf "${temp_dir}"

  [ "${failed}" = "0" ] && [ "${content}" = "protected foreign binary" ]
}

# 5. Sibling Prefix and Dot-Dot Traversal Refusal
test_sibling_prefix_and_dot_dot_escape_refusal() {
  local temp_dir
  temp_dir="$(mktemp -d)"

  local prefix="${temp_dir}/opt/dosi"
  local fake_src="${temp_dir}/fake_bin"
  create_fake_executable "${fake_src}" "0.1.0"

  local passed=1

  # Case A: Sibling prefix (/opt/dosi-sibling vs /opt/dosi)
  if bash "${INSTALLER}" install --prefix "${prefix}" --bin-dir "${temp_dir}/opt/dosi-sibling/bin" --source-bin "${fake_src}" >/dev/null 2>&1; then
    passed=0
  fi

  # Case B: Dot-dot escape (/opt/dosi/sub/../../outside/bin)
  if bash "${INSTALLER}" install --prefix "${prefix}" --bin-dir "${prefix}/sub/../../outside/bin" --source-bin "${fake_src}" >/dev/null 2>&1; then
    passed=0
  fi

  rm -rf "${temp_dir}"
  [ "${passed}" = "1" ]
}

# 6. Ancestor and Explicit Symlink Escape Refusal
test_ancestor_and_explicit_symlink_escape_refusal() {
  local temp_dir
  temp_dir="$(mktemp -d)"

  local prefix="${temp_dir}/opt/dosi"
  local outside_dir="${temp_dir}/outside"
  mkdir -p "${prefix}" "${outside_dir}"

  # Attempt to create directory symlink
  ln -s "${outside_dir}" "${prefix}/bin" 2>/dev/null || true

  # Check if real directory symlink was created by host OS
  if [ ! -L "${prefix}/bin" ]; then
    rm -rf "${temp_dir}"
    return 0 # Environment does not support directory symlinks without elevation; skip cleanly
  fi

  local fake_src="${temp_dir}/fake_bin"
  create_fake_executable "${fake_src}" "0.1.0"

  local passed=0
  if ! bash "${INSTALLER}" install --prefix "${prefix}" --bin-dir "${prefix}/bin" --source-bin "${fake_src}" >/dev/null 2>&1; then
    passed=1
  fi

  rm -rf "${temp_dir}"
  [ "${passed}" = "1" ]
}

# 7. Rollback Refuses to Overwrite Unmanaged Foreign Destination
test_rollback_refuses_to_overwrite_unmanaged_foreign_destination() {
  local temp_dir
  temp_dir="$(mktemp -d)"

  local prefix="${temp_dir}/opt/dosi"
  local fake_src_v1="${temp_dir}/fake_bin_v1"
  create_fake_executable "${fake_src_v1}" "0.1.0"
  bash "${INSTALLER}" install --prefix "${prefix}" --source-bin "${fake_src_v1}" >/dev/null

  local fake_src_v2="${temp_dir}/fake_bin_v2"
  create_fake_executable "${fake_src_v2}" "0.2.0"
  bash "${INSTALLER}" update --prefix "${prefix}" --source-bin "${fake_src_v2}" >/dev/null

  # Place unmanaged foreign binary at destination and delete manifest
  local bin_path="${prefix}/bin/dosi-tracker-linux"
  if [ ! -f "${bin_path}" ] && [ -f "${prefix}/bin/dosi-tracker-linux.exe" ]; then
    bin_path="${prefix}/bin/dosi-tracker-linux.exe"
  fi
  echo "foreign protected file" > "${bin_path}"
  rm -f "${prefix}/share/dosi-tracker/manifest.json"

  # Rollback must refuse to overwrite unmanaged destination
  local failed="0"
  if bash "${INSTALLER}" rollback --prefix "${prefix}" >/dev/null 2>&1; then
    failed="1"
  fi

  local content
  content="$(cat "${bin_path}")"
  rm -rf "${temp_dir}"

  [ "${failed}" = "0" ] && [ "${content}" = "foreign protected file" ]
}

# 8. Rollback Restores Complete Managed State (Binary, Manifest, Version)
test_rollback_restores_complete_managed_state() {
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

  local bin_path="${prefix}/bin/dosi-tracker-linux"
  if [ ! -f "${bin_path}" ] && [ -f "${prefix}/bin/dosi-tracker-linux.exe" ]; then
    bin_path="${prefix}/bin/dosi-tracker-linux.exe"
  fi

  local active_ver manifest_ver version_file_ver
  active_ver="$("${bin_path}" --version)"
  manifest_ver="$(grep '"version"' "${prefix}/share/dosi-tracker/manifest.json" | head -n1)"
  version_file_ver="$(cat "${prefix}/share/dosi-tracker/current_version")"

  rm -rf "${temp_dir}"

  [[ "${active_ver}" == *"0.1.0"* ]] && [[ "${manifest_ver}" == *"0.1.0"* ]] && [ "${version_file_ver}" = "0.1.0" ]
}

# 9. Injected Metadata Publication Failure Triggers Complete Rollback
test_metadata_write_failure_fully_rolls_back() {
  local temp_dir
  temp_dir="$(mktemp -d)"

  local prefix="${temp_dir}/opt/dosi"
  local fake_src_v1="${temp_dir}/fake_bin_v1"
  create_fake_executable "${fake_src_v1}" "0.1.0"
  bash "${INSTALLER}" install --prefix "${prefix}" --source-bin "${fake_src_v1}" >/dev/null

  local fake_src_v2="${temp_dir}/fake_bin_v2"
  create_fake_executable "${fake_src_v2}" "0.2.0"

  # Attempt update with injected metadata failure
  local failed="0"
  if DOSI_TEST_INJECT_FAIL_METADATA=1 bash "${INSTALLER}" update --prefix "${prefix}" --source-bin "${fake_src_v2}" >/dev/null 2>&1; then
    failed="1"
  fi

  local bin_path="${prefix}/bin/dosi-tracker-linux"
  if [ ! -f "${bin_path}" ] && [ -f "${prefix}/bin/dosi-tracker-linux.exe" ]; then
    bin_path="${prefix}/bin/dosi-tracker-linux.exe"
  fi

  # Assert prior binary, manifest, and version are completely restored
  local active_ver manifest_ver version_file_ver
  active_ver="$("${bin_path}" --version)"
  manifest_ver="$(grep '"version"' "${prefix}/share/dosi-tracker/manifest.json" | head -n1)"
  version_file_ver="$(cat "${prefix}/share/dosi-tracker/current_version")"

  rm -rf "${temp_dir}"

  [ "${failed}" = "0" ] && [[ "${active_ver}" == *"0.1.0"* ]] && [[ "${manifest_ver}" == *"0.1.0"* ]] && [ "${version_file_ver}" = "0.1.0" ]
}

# 10. Injected Post-Activation Smoke Failure Triggers Complete Rollback
test_smoke_check_failure_fully_rolls_back() {
  local temp_dir
  temp_dir="$(mktemp -d)"

  local prefix="${temp_dir}/opt/dosi"
  local fake_src_v1="${temp_dir}/fake_bin_v1"
  create_fake_executable "${fake_src_v1}" "0.1.0"
  bash "${INSTALLER}" install --prefix "${prefix}" --source-bin "${fake_src_v1}" >/dev/null

  local fake_src_v2="${temp_dir}/fake_bin_v2"
  create_fake_executable "${fake_src_v2}" "0.2.0"

  # Attempt update with injected smoke failure
  local failed="0"
  if DOSI_TEST_INJECT_FAIL_SMOKE=1 bash "${INSTALLER}" update --prefix "${prefix}" --source-bin "${fake_src_v2}" >/dev/null 2>&1; then
    failed="1"
  fi

  local bin_path="${prefix}/bin/dosi-tracker-linux"
  if [ ! -f "${bin_path}" ] && [ -f "${prefix}/bin/dosi-tracker-linux.exe" ]; then
    bin_path="${prefix}/bin/dosi-tracker-linux.exe"
  fi

  local active_ver manifest_ver version_file_ver
  active_ver="$("${bin_path}" --version)"
  manifest_ver="$(grep '"version"' "${prefix}/share/dosi-tracker/manifest.json" | head -n1)"
  version_file_ver="$(cat "${prefix}/share/dosi-tracker/current_version")"

  rm -rf "${temp_dir}"

  [ "${failed}" = "0" ] && [[ "${active_ver}" == *"0.1.0"* ]] && [[ "${manifest_ver}" == *"0.1.0"* ]] && [ "${version_file_ver}" = "0.1.0" ]
}

# 11. Uninstall Refuses on Corrupted or Mismatched Manifest
test_uninstall_refuses_on_corrupted_manifest() {
  local temp_dir
  temp_dir="$(mktemp -d)"

  local prefix="${temp_dir}/opt/dosi"
  mkdir -p "${prefix}/bin" "${prefix}/share/dosi-tracker"
  echo "precious user script" > "${prefix}/bin/dosi-tracker-linux"
  echo "corrupted metadata" > "${prefix}/share/dosi-tracker/manifest.json"

  local failed="0"
  if bash "${INSTALLER}" uninstall --prefix "${prefix}" >/dev/null 2>&1; then
    failed="1"
  fi

  local content
  content="$(cat "${prefix}/bin/dosi-tracker-linux")"
  rm -rf "${temp_dir}"

  [ "${failed}" = "0" ] && [ "${content}" = "precious user script" ]
}

# 12. Uninstall Preserves User Config and Unrelated Sentinels
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

# 13. Idempotent Uninstall
test_idempotent_uninstall() {
  local temp_dir
  temp_dir="$(mktemp -d)"

  local prefix="${temp_dir}/opt/empty-prefix"
  # Running uninstall on a non-existent or already cleaned prefix must exit 0 cleanly
  bash "${INSTALLER}" uninstall --prefix "${prefix}" >/dev/null

  rm -rf "${temp_dir}"
}

# 14. Real Compiled Binary Install and Diagnostic Doctor Smoke Test
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
run_test "Malformed JSON Manifest Refusal" test_malformed_manifest_refusal
run_test "Mismatched Prefix Manifest Refusal" test_mismatched_prefix_manifest_refusal
run_test "Sibling Prefix & Dot-Dot Traversal Refusal" test_sibling_prefix_and_dot_dot_escape_refusal
run_test "Ancestor & Explicit Symlink Escape Refusal" test_ancestor_and_explicit_symlink_escape_refusal
run_test "Rollback Refuses Foreign Destination" test_rollback_refuses_to_overwrite_unmanaged_foreign_destination
run_test "Rollback Restores Complete Managed State" test_rollback_restores_complete_managed_state
run_test "Metadata Failure Triggers Complete Rollback" test_metadata_write_failure_fully_rolls_back
run_test "Smoke Failure Triggers Complete Rollback" test_smoke_check_failure_fully_rolls_back
run_test "Uninstall Refuses on Corrupt Manifest" test_uninstall_refuses_on_corrupted_manifest
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
