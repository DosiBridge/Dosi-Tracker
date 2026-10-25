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
  echo "dosi-tracker ${version}"
  exit 0
elif [ "\$1" = "--help" ]; then
  echo "dosi-tracker help: diagnostic foundation"
  exit 0
elif [ "\$1" = "doctor" ]; then
  if [ "\$2" = "--json" ]; then
    echo '{"component":"dosi-tracker","version":"${version}","status":"ok"}'
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
  local bin_path="${prefix}/bin/dosi-tracker"
  if [ ! -f "${bin_path}" ] && [ -f "${prefix}/bin/dosi-tracker.exe" ]; then
    bin_path="${prefix}/bin/dosi-tracker.exe"
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
  echo "foreign proprietary payload" > "${prefix}/bin/dosi-tracker"

  local fake_src="${temp_dir}/fake_bin"
  create_fake_executable "${fake_src}" "0.1.0"

  # Install must fail and refuse to overwrite
  local failed="0"
  if bash "${INSTALLER}" install --prefix "${prefix}" --source-bin "${fake_src}" >/dev/null 2>&1; then
    failed="1"
  fi

  # Verify foreign content is completely preserved
  local content
  content="$(cat "${prefix}/bin/dosi-tracker")"

  rm -rf "${temp_dir}"

  [ "${failed}" = "0" ] && [ "${content}" = "foreign proprietary payload" ]
}

# 3. Malformed JSON Manifest Refusal (missing comma or trailing garbage, fails closed)
test_malformed_manifest_refusal() {
  local temp_dir
  temp_dir="$(mktemp -d)"

  local prefix="${temp_dir}/opt/dosi"
  mkdir -p "${prefix}/bin" "${prefix}/share/dosi-tracker"
  echo "existing payload" > "${prefix}/bin/dosi-tracker"

  local fake_src="${temp_dir}/fake_bin"
  create_fake_executable "${fake_src}" "0.1.0"

  local failed="0"

  # Case A: braces and fields present but missing comma
  cat > "${prefix}/share/dosi-tracker/manifest.json" <<EOF
{
  "format_version": 1
  "component": "dosi-tracker-linux-diagnostic-foundation",
  "version": "0.1.0",
  "installed_at": "2026-10-22T00:00:00Z",
  "prefix": "${prefix}",
  "bin_dir": "${prefix}/bin",
  "data_dir": "${prefix}/share/dosi-tracker",
  "binary_path": "${prefix}/bin/dosi-tracker",
  "owned_files": [
    "${prefix}/bin/dosi-tracker"
  ]
}
EOF
  if bash "${INSTALLER}" install --prefix "${prefix}" --source-bin "${fake_src}" >/dev/null 2>&1; then
    failed="1"
  fi

  # Case B: Trailing garbage after closing brace
  cat > "${prefix}/share/dosi-tracker/manifest.json" <<EOF
{
  "format_version": 1,
  "component": "dosi-tracker-linux-diagnostic-foundation",
  "version": "0.1.0",
  "installed_at": "2026-10-22T00:00:00Z",
  "prefix": "${prefix}",
  "bin_dir": "${prefix}/bin",
  "data_dir": "${prefix}/share/dosi-tracker",
  "binary_path": "${prefix}/bin/dosi-tracker",
  "owned_files": [
    "${prefix}/bin/dosi-tracker"
  ]
} TRAILING_GARBAGE_BYTES
EOF
  if bash "${INSTALLER}" install --prefix "${prefix}" --source-bin "${fake_src}" >/dev/null 2>&1; then
    failed="1"
  fi

  local content
  content="$(cat "${prefix}/bin/dosi-tracker")"
  rm -rf "${temp_dir}"

  [ "${failed}" = "0" ] && [ "${content}" = "existing payload" ]
}

# 4. Duplicate Keys in Manifest Refusal
test_duplicate_keys_manifest_refusal() {
  local temp_dir
  temp_dir="$(mktemp -d)"

  local prefix="${temp_dir}/opt/dosi"
  mkdir -p "${prefix}/bin" "${prefix}/share/dosi-tracker"
  echo "existing payload" > "${prefix}/bin/dosi-tracker"

  # Manifest with duplicate version key
  cat > "${prefix}/share/dosi-tracker/manifest.json" <<EOF
{
  "format_version": 1,
  "version": "0.1.0",
  "version": "0.2.0",
  "component": "dosi-tracker-linux-diagnostic-foundation",
  "installed_at": "2026-10-22T00:00:00Z",
  "prefix": "${prefix}",
  "bin_dir": "${prefix}/bin",
  "data_dir": "${prefix}/share/dosi-tracker",
  "binary_path": "${prefix}/bin/dosi-tracker",
  "owned_files": [
    "${prefix}/bin/dosi-tracker"
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
  content="$(cat "${prefix}/bin/dosi-tracker")"
  rm -rf "${temp_dir}"

  [ "${failed}" = "0" ] && [ "${content}" = "existing payload" ]
}

# 5. Invalid owned_files Structure Refusal
test_invalid_owned_files_refusal() {
  local temp_dir
  temp_dir="$(mktemp -d)"

  local prefix="${temp_dir}/opt/dosi"
  mkdir -p "${prefix}/bin" "${prefix}/share/dosi-tracker"
  echo "existing payload" > "${prefix}/bin/dosi-tracker"

  local fake_src="${temp_dir}/fake_bin"
  create_fake_executable "${fake_src}" "0.1.0"

  local failed="0"

  # Case A: Empty owned_files array
  cat > "${prefix}/share/dosi-tracker/manifest.json" <<EOF
{
  "format_version": 1,
  "component": "dosi-tracker-linux-diagnostic-foundation",
  "version": "0.1.0",
  "installed_at": "2026-10-22T00:00:00Z",
  "prefix": "${prefix}",
  "bin_dir": "${prefix}/bin",
  "data_dir": "${prefix}/share/dosi-tracker",
  "binary_path": "${prefix}/bin/dosi-tracker",
  "owned_files": []
}
EOF
  if bash "${INSTALLER}" install --prefix "${prefix}" --source-bin "${fake_src}" >/dev/null 2>&1; then
    failed="1"
  fi

  # Case B: owned_files is not an array (string)
  cat > "${prefix}/share/dosi-tracker/manifest.json" <<EOF
{
  "format_version": 1,
  "component": "dosi-tracker-linux-diagnostic-foundation",
  "version": "0.1.0",
  "installed_at": "2026-10-22T00:00:00Z",
  "prefix": "${prefix}",
  "bin_dir": "${prefix}/bin",
  "data_dir": "${prefix}/share/dosi-tracker",
  "binary_path": "${prefix}/bin/dosi-tracker",
  "owned_files": "not_an_array"
}
EOF
  if bash "${INSTALLER}" install --prefix "${prefix}" --source-bin "${fake_src}" >/dev/null 2>&1; then
    failed="1"
  fi

  # Case C: binary_path not listed in owned_files
  cat > "${prefix}/share/dosi-tracker/manifest.json" <<EOF
{
  "format_version": 1,
  "component": "dosi-tracker-linux-diagnostic-foundation",
  "version": "0.1.0",
  "installed_at": "2026-10-22T00:00:00Z",
  "prefix": "${prefix}",
  "bin_dir": "${prefix}/bin",
  "data_dir": "${prefix}/share/dosi-tracker",
  "binary_path": "${prefix}/bin/dosi-tracker",
  "owned_files": [
    "${prefix}/share/dosi-tracker/other_file"
  ]
}
EOF
  if bash "${INSTALLER}" install --prefix "${prefix}" --source-bin "${fake_src}" >/dev/null 2>&1; then
    failed="1"
  fi

  local content
  content="$(cat "${prefix}/bin/dosi-tracker")"
  rm -rf "${temp_dir}"

  [ "${failed}" = "0" ] && [ "${content}" = "existing payload" ]
}

# 6. Mismatched Prefix/Binary Manifest Refusal (fails closed)
test_mismatched_prefix_manifest_refusal() {
  local temp_dir
  temp_dir="$(mktemp -d)"

  local prefix="${temp_dir}/opt/dosi"
  mkdir -p "${prefix}/bin" "${prefix}/share/dosi-tracker"
  echo "protected foreign binary" > "${prefix}/bin/dosi-tracker"

  # Valid JSON, but prefix points to completely different directory
  cat > "${prefix}/share/dosi-tracker/manifest.json" <<EOF
{
  "format_version": 1,
  "component": "dosi-tracker-linux-diagnostic-foundation",
  "version": "0.1.0",
  "installed_at": "2026-10-22T00:00:00Z",
  "prefix": "/another/unrelated/prefix",
  "bin_dir": "/another/unrelated/prefix/bin",
  "data_dir": "/another/unrelated/prefix/share/dosi-tracker",
  "binary_path": "/another/unrelated/prefix/bin/dosi-tracker",
  "owned_files": [
    "/another/unrelated/prefix/bin/dosi-tracker"
  ],
  "previous_version": null
}
EOF

  local fake_src="${temp_dir}/fake_bin"
  create_fake_executable "${fake_src}" "0.1.0"

  local failed="0"
  if bash "${INSTALLER}" install --prefix "${prefix}" --source-bin "${fake_src}" >/dev/null 2>&1; then
    failed="1"
  fi

  local content
  content="$(cat "${prefix}/bin/dosi-tracker")"
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
  local bin_path="${prefix}/bin/dosi-tracker"
  if [ ! -f "${bin_path}" ] && [ -f "${prefix}/bin/dosi-tracker.exe" ]; then
    bin_path="${prefix}/bin/dosi-tracker.exe"
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

# 10. Rollback Refuses Foreign-Prefix or Mismatched Backup (Preserves Current State & Sentinels)
test_rollback_refuses_foreign_prefix_or_mismatched_backup() {
  local temp_dir
  temp_dir="$(mktemp -d)"

  local prefix="${temp_dir}/opt/dosi"
  local fake_src1="${temp_dir}/fake_bin_1"
  create_fake_executable "${fake_src1}" "1.0.0"

  # Fresh install v1.0.0
  bash "${INSTALLER}" install --prefix "${prefix}" --source-bin "${fake_src1}" >/dev/null

  # Place user config sentinel
  local config_dir="${temp_dir}/home/.config/dosi-tracker"
  mkdir -p "${config_dir}"
  echo "precious_user_consent" > "${config_dir}/consent.json"

  # Place foreign sentinel file
  echo "unrelated_sentinel_data" > "${prefix}/share/unrelated.txt"

  # Now craft a backup manifest with a foreign prefix
  local backup_dir="${prefix}/share/dosi-tracker/backup"
  mkdir -p "${backup_dir}"
  local backup_bin="${backup_dir}/dosi-tracker.prev"
  create_fake_executable "${backup_bin}" "0.9.0"

  # Backup manifest is valid JSON, but has prefix /foreign/opt/dosi
  cat > "${backup_dir}/manifest.json.prev" <<EOF
{
  "format_version": 1,
  "component": "dosi-tracker-linux-diagnostic-foundation",
  "version": "0.9.0",
  "installed_at": "2026-10-22T00:00:00Z",
  "prefix": "/foreign/opt/dosi",
  "bin_dir": "/foreign/opt/dosi/bin",
  "data_dir": "/foreign/opt/dosi/share/dosi-tracker",
  "binary_path": "/foreign/opt/dosi/bin/dosi-tracker",
  "owned_files": [
    "/foreign/opt/dosi/bin/dosi-tracker"
  ],
  "previous_version": null
}
EOF
  echo "0.9.0" > "${backup_dir}/current_version.prev"

  # Record current state before rollback attempt
  local bin_path="${prefix}/bin/dosi-tracker"
  if [ ! -f "${bin_path}" ] && [ -f "${prefix}/bin/dosi-tracker.exe" ]; then
    bin_path="${prefix}/bin/dosi-tracker.exe"
  fi

  local cur_bin_content cur_manifest_content cur_version_content
  cur_bin_content="$("${bin_path}" --version)"
  cur_manifest_content="$(cat "${prefix}/share/dosi-tracker/manifest.json")"
  cur_version_content="$(cat "${prefix}/share/dosi-tracker/current_version")"

  # Attempt rollback: MUST FAIL because backup manifest is bound to /foreign/opt/dosi
  local failed="0"
  if bash "${INSTALLER}" rollback --prefix "${prefix}" >/dev/null 2>&1; then
    failed="1"
  fi

  # Verify current binary, manifest, and version are 100% UNCHANGED
  local after_bin_content after_manifest_content after_version_content
  after_bin_content="$("${bin_path}" --version)"
  after_manifest_content="$(cat "${prefix}/share/dosi-tracker/manifest.json")"
  after_version_content="$(cat "${prefix}/share/dosi-tracker/current_version")"

  # Verify config and sentinels are completely intact
  local after_config after_sentinel
  after_config="$(cat "${config_dir}/consent.json")"
  after_sentinel="$(cat "${prefix}/share/unrelated.txt")"

  rm -rf "${temp_dir}"

  [ "${failed}" = "0" ] && \
  [ "${cur_bin_content}" = "${after_bin_content}" ] && \
  [ "${cur_manifest_content}" = "${after_manifest_content}" ] && \
  [ "${cur_version_content}" = "${after_version_content}" ] && \
  [ "${after_config}" = "precious_user_consent" ] && \
  [ "${after_sentinel}" = "unrelated_sentinel_data" ]
}

# 11. Rollback Restores Complete Managed State (Binary, Manifest, Version)
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

  local bin_path="${prefix}/bin/dosi-tracker"
  if [ ! -f "${bin_path}" ] && [ -f "${prefix}/bin/dosi-tracker.exe" ]; then
    bin_path="${prefix}/bin/dosi-tracker.exe"
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

  local bin_path="${prefix}/bin/dosi-tracker"
  if [ ! -f "${bin_path}" ] && [ -f "${prefix}/bin/dosi-tracker.exe" ]; then
    bin_path="${prefix}/bin/dosi-tracker.exe"
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

  local bin_path="${prefix}/bin/dosi-tracker"
  if [ ! -f "${bin_path}" ] && [ -f "${prefix}/bin/dosi-tracker.exe" ]; then
    bin_path="${prefix}/bin/dosi-tracker.exe"
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
  echo "precious user script" > "${prefix}/bin/dosi-tracker"
  echo "corrupted metadata" > "${prefix}/share/dosi-tracker/manifest.json"

  local failed="0"
  if bash "${INSTALLER}" uninstall --prefix "${prefix}" >/dev/null 2>&1; then
    failed="1"
  fi

  local content
  content="$(cat "${prefix}/bin/dosi-tracker")"
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
  if [ -f "${prefix}/bin/dosi-tracker" ] || [ -f "${prefix}/bin/dosi-tracker.exe" ]; then
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

  local bin_path="${prefix}/bin/dosi-tracker"
  if [ ! -f "${bin_path}" ] && [ -f "${prefix}/bin/dosi-tracker.exe" ]; then
    bin_path="${prefix}/bin/dosi-tracker.exe"
  fi
  [ -f "${bin_path}" ]

  # Run installed diagnostic doctor --json
  local doctor_json
  doctor_json="$("${bin_path}" doctor --json)"

  rm -rf "${temp_dir}"

  # Assert JSON contains truthful diagnoses
  [[ "${doctor_json}" == *"dosi-tracker-linux"* ]] && [[ "${doctor_json}" == *"NOT_IMPLEMENTED"* ]]
}

# 15. Extra Unrelated In-Prefix Sentinel in owned_files Refusal
test_uninstall_refuses_extra_in_prefix_sentinel() {
  local temp_dir
  temp_dir="$(mktemp -d)"

  local prefix="${temp_dir}/opt/dosi"
  local fake_src="${temp_dir}/fake_bin"
  create_fake_executable "${fake_src}" "0.1.0"

  bash "${INSTALLER}" install --prefix "${prefix}" --source-bin "${fake_src}" >/dev/null

  # Create unrelated in-prefix sentinel and outside foreign sentinel
  local unrelated_sentinel="${prefix}/unrelated_sentinel.txt"
  echo "unrelated sentinel payload" > "${unrelated_sentinel}"

  local foreign_sentinel="${temp_dir}/foreign_sentinel.txt"
  echo "foreign payload" > "${foreign_sentinel}"

  local mock_home="${temp_dir}/mock_home"
  mkdir -p "${mock_home}/.config/dosi-tracker"
  echo '{"user_consent": true}' > "${mock_home}/.config/dosi-tracker/config.json"

  # Inject unauthorized in-prefix file into manifest owned_files alongside legitimate binary
  local bin_path="${prefix}/bin/dosi-tracker"
  if [ -f "${prefix}/bin/dosi-tracker.exe" ]; then
    bin_path="${prefix}/bin/dosi-tracker.exe"
  fi

  cat > "${prefix}/share/dosi-tracker/manifest.json" <<EOF
{
  "format_version": 1,
  "component": "dosi-tracker-linux-diagnostic-foundation",
  "version": "0.1.0",
  "installed_at": "2026-10-22T00:00:00Z",
  "prefix": "${prefix}",
  "bin_dir": "${prefix}/bin",
  "data_dir": "${prefix}/share/dosi-tracker",
  "binary_path": "${bin_path}",
  "owned_files": [
    "${bin_path}",
    "${prefix}/share/dosi-tracker/manifest.json",
    "${prefix}/share/dosi-tracker/current_version",
    "${unrelated_sentinel}"
  ],
  "previous_version": null
}
EOF

  local failed="0"
  if HOME="${mock_home}" bash "${INSTALLER}" uninstall --prefix "${prefix}" >/dev/null 2>&1; then
    failed="1"
  fi

  # Assert command rejected BEFORE ANY mutation
  local bin_ok=0
  local manifest_ok=0
  local version_ok=0
  local unrelated_ok=0
  local foreign_ok=0
  local config_ok=0

  [ -f "${bin_path}" ] && bin_ok=1
  [ -f "${prefix}/share/dosi-tracker/manifest.json" ] && manifest_ok=1
  [ -f "${prefix}/share/dosi-tracker/current_version" ] && version_ok=1
  [ -f "${unrelated_sentinel}" ] && [ "$(cat "${unrelated_sentinel}")" = "unrelated sentinel payload" ] && unrelated_ok=1
  [ -f "${foreign_sentinel}" ] && [ "$(cat "${foreign_sentinel}")" = "foreign payload" ] && foreign_ok=1
  [ -f "${mock_home}/.config/dosi-tracker/config.json" ] && config_ok=1

  rm -rf "${temp_dir}"

  [ "${failed}" = "0" ] && [ "${bin_ok}" = "1" ] && [ "${manifest_ok}" = "1" ] && \
  [ "${version_ok}" = "1" ] && [ "${unrelated_ok}" = "1" ] && [ "${foreign_ok}" = "1" ] && [ "${config_ok}" = "1" ]
}

# 16. Traversal ../ Foreign Sentinel in owned_files Refusal
test_uninstall_refuses_traversal_foreign_sentinel() {
  local temp_dir
  temp_dir="$(mktemp -d)"

  local prefix="${temp_dir}/opt/dosi"
  local fake_src="${temp_dir}/fake_bin"
  create_fake_executable "${fake_src}" "0.1.0"

  bash "${INSTALLER}" install --prefix "${prefix}" --source-bin "${fake_src}" >/dev/null

  local foreign_sentinel="${temp_dir}/foreign_sentinel.txt"
  echo "foreign payload" > "${foreign_sentinel}"

  local bin_path="${prefix}/bin/dosi-tracker"
  if [ -f "${prefix}/bin/dosi-tracker.exe" ]; then
    bin_path="${prefix}/bin/dosi-tracker.exe"
  fi

  # Inject ../ traversal entry into owned_files alongside legitimate binary
  cat > "${prefix}/share/dosi-tracker/manifest.json" <<EOF
{
  "format_version": 1,
  "component": "dosi-tracker-linux-diagnostic-foundation",
  "version": "0.1.0",
  "installed_at": "2026-10-22T00:00:00Z",
  "prefix": "${prefix}",
  "bin_dir": "${prefix}/bin",
  "data_dir": "${prefix}/share/dosi-tracker",
  "binary_path": "${bin_path}",
  "owned_files": [
    "${bin_path}",
    "${prefix}/share/dosi-tracker/manifest.json",
    "${prefix}/share/dosi-tracker/current_version",
    "${prefix}/../../foreign_sentinel.txt"
  ],
  "previous_version": null
}
EOF

  local failed="0"
  if bash "${INSTALLER}" uninstall --prefix "${prefix}" >/dev/null 2>&1; then
    failed="1"
  fi

  local foreign_ok=0
  local bin_ok=0
  [ -f "${foreign_sentinel}" ] && [ "$(cat "${foreign_sentinel}")" = "foreign payload" ] && foreign_ok=1
  [ -f "${bin_path}" ] && bin_ok=1

  rm -rf "${temp_dir}"

  [ "${failed}" = "0" ] && [ "${foreign_ok}" = "1" ] && [ "${bin_ok}" = "1" ]
}

# 17. Symlink Parent Escape in owned_files Refusal
test_uninstall_refuses_symlink_parent_escape() {
  local temp_dir
  temp_dir="$(mktemp -d)"

  local prefix="${temp_dir}/opt/dosi"
  local fake_src="${temp_dir}/fake_bin"
  create_fake_executable "${fake_src}" "0.1.0"

  bash "${INSTALLER}" install --prefix "${prefix}" --source-bin "${fake_src}" >/dev/null

  local outside_dir="${temp_dir}/outside"
  mkdir -p "${outside_dir}"
  local secret_file="${outside_dir}/secret.txt"
  echo "secret payload" > "${secret_file}"

  ln -s "${outside_dir}" "${prefix}/link_dir" 2>/dev/null || true

  local bin_path="${prefix}/bin/dosi-tracker"
  if [ -f "${prefix}/bin/dosi-tracker.exe" ]; then
    bin_path="${prefix}/bin/dosi-tracker.exe"
  fi

  cat > "${prefix}/share/dosi-tracker/manifest.json" <<EOF
{
  "format_version": 1,
  "component": "dosi-tracker-linux-diagnostic-foundation",
  "version": "0.1.0",
  "installed_at": "2026-10-22T00:00:00Z",
  "prefix": "${prefix}",
  "bin_dir": "${prefix}/bin",
  "data_dir": "${prefix}/share/dosi-tracker",
  "binary_path": "${bin_path}",
  "owned_files": [
    "${bin_path}",
    "${prefix}/share/dosi-tracker/manifest.json",
    "${prefix}/share/dosi-tracker/current_version",
    "${prefix}/link_dir/secret.txt"
  ],
  "previous_version": null
}
EOF

  local failed="0"
  if bash "${INSTALLER}" uninstall --prefix "${prefix}" >/dev/null 2>&1; then
    failed="1"
  fi

  local secret_ok=0
  local bin_ok=0
  [ -f "${secret_file}" ] && [ "$(cat "${secret_file}")" = "secret payload" ] && secret_ok=1
  [ -f "${bin_path}" ] && bin_ok=1

  rm -rf "${temp_dir}"

  [ "${failed}" = "0" ] && [ "${secret_ok}" = "1" ] && [ "${bin_ok}" = "1" ]
}

# 18. Embedded Newline and Whitespace Confusion in owned_files Refusal
test_uninstall_refuses_embedded_newline_and_whitespace_confusion() {
  local temp_dir
  temp_dir="$(mktemp -d)"

  local prefix="${temp_dir}/opt/dosi"
  local fake_src="${temp_dir}/fake_bin"
  create_fake_executable "${fake_src}" "0.1.0"

  bash "${INSTALLER}" install --prefix "${prefix}" --source-bin "${fake_src}" >/dev/null

  local secret_file="${prefix}/target_secret.txt"
  echo "target secret payload" > "${secret_file}"

  local bin_path="${prefix}/bin/dosi-tracker"
  if [ -f "${prefix}/bin/dosi-tracker.exe" ]; then
    bin_path="${prefix}/bin/dosi-tracker.exe"
  fi

  # Case A: Embedded newline path confusion
  cat > "${prefix}/share/dosi-tracker/manifest.json" <<EOF
{
  "format_version": 1,
  "component": "dosi-tracker-linux-diagnostic-foundation",
  "version": "0.1.0",
  "installed_at": "2026-10-22T00:00:00Z",
  "prefix": "${prefix}",
  "bin_dir": "${prefix}/bin",
  "data_dir": "${prefix}/share/dosi-tracker",
  "binary_path": "${bin_path}",
  "owned_files": [
    "${bin_path}",
    "${prefix}/share/dosi-tracker/manifest.json",
    "${prefix}/share/dosi-tracker/current_version",
    "${bin_path}\n${secret_file}"
  ],
  "previous_version": null
}
EOF

  local failed_nl="0"
  if bash "${INSTALLER}" uninstall --prefix "${prefix}" >/dev/null 2>&1; then
    failed_nl="1"
  fi

  # Case B: Leading/trailing whitespace in path
  cat > "${prefix}/share/dosi-tracker/manifest.json" <<EOF
{
  "format_version": 1,
  "component": "dosi-tracker-linux-diagnostic-foundation",
  "version": "0.1.0",
  "installed_at": "2026-10-22T00:00:00Z",
  "prefix": "${prefix}",
  "bin_dir": "${prefix}/bin",
  "data_dir": "${prefix}/share/dosi-tracker",
  "binary_path": "${bin_path}",
  "owned_files": [
    "  ${bin_path}  ",
    "${prefix}/share/dosi-tracker/manifest.json",
    "${prefix}/share/dosi-tracker/current_version"
  ],
  "previous_version": null
}
EOF

  local failed_ws="0"
  if bash "${INSTALLER}" uninstall --prefix "${prefix}" >/dev/null 2>&1; then
    failed_ws="1"
  fi

  local secret_ok=0
  local bin_ok=0
  [ -f "${secret_file}" ] && [ "$(cat "${secret_file}")" = "target secret payload" ] && secret_ok=1
  [ -f "${bin_path}" ] && bin_ok=1

  rm -rf "${temp_dir}"

  [ "${failed_nl}" = "0" ] && [ "${failed_ws}" = "0" ] && [ "${secret_ok}" = "1" ] && [ "${bin_ok}" = "1" ]
}

# 19. Update and Rollback Refusal on Unauthorized Extra owned_files
test_update_and_rollback_refuse_unauthorized_owned_files() {
  local temp_dir
  temp_dir="$(mktemp -d)"

  local prefix="${temp_dir}/opt/dosi"
  local fake_src1="${temp_dir}/fake_bin_1"
  create_fake_executable "${fake_src1}" "0.1.0"

  bash "${INSTALLER}" install --prefix "${prefix}" --source-bin "${fake_src1}" >/dev/null

  local unrelated_file="${prefix}/unrelated_sentinel.txt"
  echo "preserve unrelated" > "${unrelated_file}"

  local bin_path="${prefix}/bin/dosi-tracker"
  if [ -f "${prefix}/bin/dosi-tracker.exe" ]; then
    bin_path="${prefix}/bin/dosi-tracker.exe"
  fi

  # Inject unauthorized file into existing live manifest
  cat > "${prefix}/share/dosi-tracker/manifest.json" <<EOF
{
  "format_version": 1,
  "component": "dosi-tracker-linux-diagnostic-foundation",
  "version": "0.1.0",
  "installed_at": "2026-10-22T00:00:00Z",
  "prefix": "${prefix}",
  "bin_dir": "${prefix}/bin",
  "data_dir": "${prefix}/share/dosi-tracker",
  "binary_path": "${bin_path}",
  "owned_files": [
    "${bin_path}",
    "${prefix}/share/dosi-tracker/manifest.json",
    "${prefix}/share/dosi-tracker/current_version",
    "${unrelated_file}"
  ],
  "previous_version": null
}
EOF

  local fake_src2="${temp_dir}/fake_bin_2"
  create_fake_executable "${fake_src2}" "0.2.0"

  # Update must refuse before touching live files
  local update_failed="0"
  if bash "${INSTALLER}" update --prefix "${prefix}" --source-bin "${fake_src2}" >/dev/null 2>&1; then
    update_failed="1"
  fi

  # Now fix live manifest and perform legitimate update to create backup
  cat > "${prefix}/share/dosi-tracker/manifest.json" <<EOF
{
  "format_version": 1,
  "component": "dosi-tracker-linux-diagnostic-foundation",
  "version": "0.1.0",
  "installed_at": "2026-10-22T00:00:00Z",
  "prefix": "${prefix}",
  "bin_dir": "${prefix}/bin",
  "data_dir": "${prefix}/share/dosi-tracker",
  "binary_path": "${bin_path}",
  "owned_files": [
    "${bin_path}",
    "${prefix}/share/dosi-tracker/manifest.json",
    "${prefix}/share/dosi-tracker/current_version"
  ],
  "previous_version": null
}
EOF

  bash "${INSTALLER}" update --prefix "${prefix}" --source-bin "${fake_src2}" >/dev/null

  # Inject unauthorized entry into backup manifest
  local backup_manifest="${prefix}/share/dosi-tracker/backup/manifest.json.prev"
  cat > "${backup_manifest}" <<EOF
{
  "format_version": 1,
  "component": "dosi-tracker-linux-diagnostic-foundation",
  "version": "0.1.0",
  "installed_at": "2026-10-22T00:00:00Z",
  "prefix": "${prefix}",
  "bin_dir": "${prefix}/bin",
  "data_dir": "${prefix}/share/dosi-tracker",
  "binary_path": "${bin_path}",
  "owned_files": [
    "${bin_path}",
    "${prefix}/share/dosi-tracker/manifest.json",
    "${prefix}/share/dosi-tracker/current_version",
    "${unrelated_file}"
  ],
  "previous_version": null
}
EOF

  # Rollback must refuse corrupted backup before touching destination
  local rollback_failed="0"
  if bash "${INSTALLER}" rollback --prefix "${prefix}" >/dev/null 2>&1; then
    rollback_failed="1"
  fi

  local current_ver
  current_ver="$(cat "${prefix}/share/dosi-tracker/current_version")"
  local unrelated_ok=0
  [ -f "${unrelated_file}" ] && [ "$(cat "${unrelated_file}")" = "preserve unrelated" ] && unrelated_ok=1

  rm -rf "${temp_dir}"

  [ "${update_failed}" = "0" ] && [ "${rollback_failed}" = "0" ] && [ "${current_ver}" = "0.2.0" ] && [ "${unrelated_ok}" = "1" ]
}

echo "=== Running Dosi-Tracker Linux Installer Lifecycle Tests ==="
run_test "Fresh Install to Isolated Prefix" test_fresh_install_to_isolated_temp_prefix
run_test "Foreign File Conflict Refusal" test_foreign_file_conflict_refusal
run_test "Malformed JSON Manifest Refusal" test_malformed_manifest_refusal
run_test "Duplicate Keys Manifest Refusal" test_duplicate_keys_manifest_refusal
run_test "Invalid owned_files Structure Refusal" test_invalid_owned_files_refusal
run_test "Extra In-Prefix Sentinel in owned_files Refusal" test_uninstall_refuses_extra_in_prefix_sentinel
run_test "Traversal ../ Foreign Sentinel in owned_files Refusal" test_uninstall_refuses_traversal_foreign_sentinel
run_test "Symlink Parent Escape in owned_files Refusal" test_uninstall_refuses_symlink_parent_escape
run_test "Embedded Newline & Whitespace Confusion Refusal" test_uninstall_refuses_embedded_newline_and_whitespace_confusion
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
run_test "Real Compiled Binary Smoke & Doctor" test_real_compiled_binary_install_smoke

echo "=== Test Summary ==="
echo "Passed: ${PASSED_TESTS}"
echo "Failed: ${FAILED_TESTS}"

if [ "${FAILED_TESTS}" -gt 0 ]; then
  exit 1
fi
exit 0
