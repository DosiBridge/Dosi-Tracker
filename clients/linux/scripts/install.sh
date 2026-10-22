#!/usr/bin/env bash
#
# Rootless, unprivileged source-build installer, updater, and uninstaller
# for Dosi-Tracker Linux Desktop Client (Headless Diagnostic Foundation).
#
# STRICT INVARIANTS:
# 1. Strictly rootless: refuses sudo/root execution; installs into user prefix.
# 2. Strict prefix confinement: validates exact path boundary (no sibling-prefix matches),
#    checks dot-dot normalization, and verifies all existing ancestors against symlink escapes.
# 3. Ownership manifest: strict structured validation of format_version (1), component,
#    version, and exact canonical bindings for prefix, bin_dir, data_dir, and binary_path.
# 4. Fail-closed conflict protection: refuses to overwrite or delete unmanaged foreign files
#    or files governed by malformed/mismatched/escaping manifests.
# 5. Full staging: stages candidate binary, manifest, and version together; validates
#    structured metadata before any activation.
# 6. Atomic failure recovery: if activation, metadata publication, or post-activation smoke
#    fails, updates restore prior binary, manifest, and version from backup completely.
# 7. Rollback validation: verifies destination manifest ownership and backup integrity
#    before restoring binary, manifest, and version.
# 8. Safe uninstall: deletes ONLY files recorded in the ownership manifest;
#    strictly preserves user configuration and consent (~/.config/dosi-tracker) and unrelated sentinels.
# 9. Sensor-free: does NOT install autostart services, D-Bus hooks, kernel modules, or sensors.
#

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
LINUX_CRATE_DIR="$(cd "${SCRIPT_DIR}/.." && pwd -P)"

# Read crate version from Cargo.toml
CRATE_VERSION="$(grep '^version' "${LINUX_CRATE_DIR}/Cargo.toml" | head -n1 | sed -E 's/version[[:space:]]*=[[:space:]]*"([^"]+)"/\1/')"
if [ -z "${CRATE_VERSION}" ]; then
  CRATE_VERSION="0.1.0"
fi

COMPONENT_NAME="dosi-tracker-linux-diagnostic-foundation"
DEFAULT_PREFIX="${HOME}/.local"

# Defaults
PREFIX="${PREFIX:-${DEFAULT_PREFIX}}"
BIN_DIR=""
DATA_DIR=""
STAGING_DIR=""
SOURCE_BIN_OVERRIDE="${DOSI_INSTALL_SOURCE_BIN:-}"
ALLOW_ROOT="${DOSI_ALLOW_ROOT_FOR_TEST:-0}"
COMMAND=""

cleanup_staging() {
  if [ -n "${STAGING_DIR:-}" ] && [ -d "${STAGING_DIR:-}" ]; then
    rm -rf "${STAGING_DIR}" 2>/dev/null || true
  fi
}
trap cleanup_staging EXIT

usage() {
  cat <<EOF
Dosi-Tracker Linux Source-Build Installer (${CRATE_VERSION})

USAGE:
    install.sh [SUBCOMMAND] [OPTIONS]

SUBCOMMANDS:
    install     Build and install the diagnostic foundation (default action)
    update      Update an existing installation with staging and failure rollback
    rollback    Restore previous working binary from backup if available
    uninstall   Remove installed binary and manifest (preserves user config/consent)
    status      Show current installation status and manifest metadata

OPTIONS:
    --prefix <DIR>        Rootless installation prefix (default: ~/.local)
    --bin-dir <DIR>       Directory for executable binary (default: <PREFIX>/bin)
    --data-dir <DIR>      Directory for metadata/manifest (default: <PREFIX>/share/dosi-tracker)
    --source-bin <PATH>   Path to pre-compiled binary (overrides local cargo build)
    --allow-root          Allow running as root (strictly for automated isolated test fixtures)
    -h, --help            Show this help message
    -v, --version         Show installer version

INVARIANTS:
    - Rootless only: never requires or runs with sudo or root privileges.
    - Preserves user consent and configuration at ~/.config/dosi-tracker upon uninstall.
    - Sensor-free: installs diagnostic foundation executable only; no background daemons.
EOF
}

# Parse CLI arguments
while [[ $# -gt 0 ]]; do
  case "$1" in
    install|update|rollback|uninstall|status)
      if [ -n "${COMMAND}" ]; then
        echo "Error: Multiple subcommands specified ('${COMMAND}' and '$1')" >&2
        exit 1
      fi
      COMMAND="$1"
      shift
      ;;
    --prefix)
      PREFIX="$2"
      shift 2
      ;;
    --prefix=*)
      PREFIX="${1#*=}"
      shift
      ;;
    --bin-dir)
      BIN_DIR="$2"
      shift 2
      ;;
    --bin-dir=*)
      BIN_DIR="${1#*=}"
      shift
      ;;
    --data-dir)
      DATA_DIR="$2"
      shift 2
      ;;
    --data-dir=*)
      DATA_DIR="${1#*=}"
      shift
      ;;
    --source-bin)
      SOURCE_BIN_OVERRIDE="$2"
      shift 2
      ;;
    --source-bin=*)
      SOURCE_BIN_OVERRIDE="${1#*=}"
      shift
      ;;
    --allow-root)
      ALLOW_ROOT="1"
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    -v|--version)
      echo "dosi-tracker-linux installer version ${CRATE_VERSION}"
      exit 0
      ;;
    *)
      echo "Error: Unknown argument '$1'. Run with --help for usage." >&2
      exit 1
      ;;
  esac
done

if [ -z "${COMMAND}" ]; then
  COMMAND="install"
fi

# 1. Root / Sudo Check
if [ "${ALLOW_ROOT}" != "1" ]; then
  EUID_VAL="${EUID:-$(id -u 2>/dev/null || echo 1000)}"
  USER_VAL="${USER:-$(whoami 2>/dev/null || echo unknown)}"
  if [ "${EUID_VAL}" -eq 0 ] || [ "${USER_VAL}" = "root" ]; then
    echo "Error: Refusing to run as root/sudo." >&2
    echo "Dosi-Tracker Linux client is strictly rootless and unprivileged." >&2
    echo "Installing into system directories or running with elevated privileges is forbidden." >&2
    exit 1
  fi
fi

# 2. Path Helpers & Prefix Confinement

# Normalize path without resolving symlinks (handles relative components and dot-dot lexically)
normalize_path_lexical() {
  local raw="$1"
  if [ -z "$raw" ]; then
    echo ""
    return 0
  fi

  # Prepend current working directory if relative
  if [[ "$raw" != /* ]]; then
    raw="$(pwd -P)/${raw}"
  fi

  # Normalize slashes
  raw="$(printf '%s' "${raw}" | sed -E 's|/+|/|g')"

  local IFS='/'
  read -ra segments <<< "${raw}"
  local stack=()

  for seg in "${segments[@]}"; do
    if [ -z "$seg" ] || [ "$seg" = "." ]; then
      continue
    elif [ "$seg" = ".." ]; then
      if [ ${#stack[@]} -gt 0 ]; then
        unset 'stack[${#stack[@]}-1]'
      fi
    else
      stack+=("$seg")
    fi
  done

  local result=""
  for seg in "${stack[@]}"; do
    result="${result}/${seg}"
  done

  if [ -z "$result" ]; then
    result="/"
  fi
  printf '%s' "${result}"
}

# Strict boundary check: returns 0 if target is inside or equal to prefix with path separator boundary
is_path_inside_prefix() {
  local target="$1"
  local prefix="$2"

  if [ -z "$target" ] || [ -z "$prefix" ]; then
    return 1
  fi

  local clean_target clean_prefix
  clean_target="$(printf '%s' "${target%/}")"
  clean_prefix="$(printf '%s' "${prefix%/}")"

  if [ "${clean_target}" = "${clean_prefix}" ]; then
    return 0
  fi

  local prefix_with_slash="${clean_prefix}/"
  if [[ "${clean_target}" == "${prefix_with_slash}"* ]]; then
    return 0
  fi

  return 1
}

# Verify path safety, blacklist, and strict prefix confinement
verify_path_confinement() {
  local target="$1"
  local expected_prefix="$2"
  local label="$3"

  local norm_target
  norm_target="$(normalize_path_lexical "$target")"

  case "$norm_target" in
    /|/bin|/sbin|/usr|/usr/bin|/usr/sbin|/etc|/var|/lib|/lib64|/root|/sys|/proc|/dev|/boot)
      echo "Error: Refusing to install into system-critical directory: ${norm_target}" >&2
      exit 1
      ;;
    /usr/*)
      echo "Error: System directory '${norm_target}' is forbidden for ${label}. Use a user-owned prefix like ~/.local." >&2
      exit 1
      ;;
    /etc/*|/var/*|/root/*|/sys/*|/proc/*|/dev/*)
      echo "Error: Protected system path '${norm_target}' is forbidden for ${label}." >&2
      exit 1
      ;;
  esac

  # Verify lexical containment first (catches sibling prefixes and dot-dot escapes)
  if ! is_path_inside_prefix "$norm_target" "$expected_prefix"; then
    echo "Error: ${label} '${target}' (normalized: '${norm_target}') is outside selected prefix '${expected_prefix}'." >&2
    exit 1
  fi

  # Walk down path components to detect ancestor symlink escapes before directory creation
  local current=""
  local IFS='/'
  read -ra segments <<< "${norm_target}"
  for seg in "${segments[@]}"; do
    if [ -z "$seg" ]; then
      continue
    fi
    current="${current}/${seg}"
    if [ -L "$current" ]; then
      local link_dest
      link_dest="$(cd -P "$(dirname "$current")" 2>/dev/null && cd -P "$current" 2>/dev/null && pwd -P || true)"
      if [ -z "$link_dest" ]; then
        link_dest="$(readlink -f "$current" 2>/dev/null || readlink "$current" 2>/dev/null || true)"
      fi
      if [ -n "$link_dest" ]; then
        if ! is_path_inside_prefix "$link_dest" "$expected_prefix"; then
          echo "Error: Symlink escape detected in ${label}: '${current}' points outside prefix to '${link_dest}'." >&2
          exit 1
        fi
      fi
    fi
  done
}

# Resolve canonical prefix
if [ -z "${PREFIX}" ]; then
  echo "Error: Prefix cannot be empty." >&2
  exit 1
fi

NORM_PREFIX="$(normalize_path_lexical "${PREFIX}")"
case "$NORM_PREFIX" in
  /|/bin|/sbin|/usr|/usr/bin|/usr/sbin|/etc|/var|/lib|/lib64|/root|/sys|/proc|/dev|/boot)
    echo "Error: Refusing to install into system-critical directory: ${NORM_PREFIX}" >&2
    exit 1
    ;;
  /usr/*|/etc/*|/var/*|/root/*|/sys/*|/proc/*|/dev/*)
    echo "Error: Protected system path '${NORM_PREFIX}' is forbidden as installation prefix." >&2
    exit 1
    ;;
esac

# Create and canonicalize prefix
mkdir -p "${NORM_PREFIX}" 2>/dev/null || true
CANONICAL_PREFIX="$(cd -P "${NORM_PREFIX}" 2>/dev/null && pwd -P || echo "${NORM_PREFIX}")"

# Resolve and verify BIN_DIR
if [ -z "${BIN_DIR}" ]; then
  RAW_BIN_DIR="${CANONICAL_PREFIX}/bin"
else
  RAW_BIN_DIR="${BIN_DIR}"
fi
verify_path_confinement "${RAW_BIN_DIR}" "${CANONICAL_PREFIX}" "Binary directory"
mkdir -p "${RAW_BIN_DIR}" 2>/dev/null || true
BIN_DIR="$(cd -P "${RAW_BIN_DIR}" 2>/dev/null && pwd -P || echo "${RAW_BIN_DIR}")"
if ! is_path_inside_prefix "${BIN_DIR}" "${CANONICAL_PREFIX}"; then
  echo "Error: Resolved binary directory '${BIN_DIR}' escapes prefix '${CANONICAL_PREFIX}'." >&2
  exit 1
fi

# Resolve and verify DATA_DIR
if [ -z "${DATA_DIR}" ]; then
  RAW_DATA_DIR="${CANONICAL_PREFIX}/share/dosi-tracker"
else
  RAW_DATA_DIR="${DATA_DIR}"
fi
verify_path_confinement "${RAW_DATA_DIR}" "${CANONICAL_PREFIX}" "Data directory"
mkdir -p "${RAW_DATA_DIR}" 2>/dev/null || true
DATA_DIR="$(cd -P "${RAW_DATA_DIR}" 2>/dev/null && pwd -P || echo "${RAW_DATA_DIR}")"
if ! is_path_inside_prefix "${DATA_DIR}" "${CANONICAL_PREFIX}"; then
  echo "Error: Resolved data directory '${DATA_DIR}' escapes prefix '${CANONICAL_PREFIX}'." >&2
  exit 1
fi

TARGET_BIN_NAME="dosi-tracker-linux"
TARGET_BIN_PATH="${BIN_DIR}/${TARGET_BIN_NAME}"
TARGET_BIN_EXE_PATH="${BIN_DIR}/${TARGET_BIN_NAME}.exe"
MANIFEST_PATH="${DATA_DIR}/manifest.json"
CURRENT_VERSION_PATH="${DATA_DIR}/current_version"
BACKUP_DIR="${DATA_DIR}/backup"
BACKUP_BIN_PATH="${BACKUP_DIR}/${TARGET_BIN_NAME}.prev"
BACKUP_BIN_EXE_PATH="${BACKUP_DIR}/${TARGET_BIN_NAME}.exe.prev"
BACKUP_MANIFEST_PATH="${BACKUP_DIR}/manifest.json.prev"
BACKUP_VERSION_PATH="${BACKUP_DIR}/current_version.prev"

# 3. Structured JSON Manifest Parser & Validator (No eval/source)

extract_json_string() {
  local key="$1"
  local file="$2"
  sed -n -E "s/^[[:space:]]*\"${key}\"[[:space:]]*:[[:space:]]*\"([^\"]*)\".*/\1/p" "${file}" 2>/dev/null | head -n1
}

extract_json_int() {
  local key="$1"
  local file="$2"
  sed -n -E "s/^[[:space:]]*\"${key}\"[[:space:]]*:[[:space:]]*([0-9]+).*/\1/p" "${file}" 2>/dev/null | head -n1
}

validate_manifest_structure() {
  local file="$1"
  if [ ! -f "${file}" ]; then
    return 1
  fi

  # Symlink check on manifest file: must not escape DATA_DIR
  if [ -L "${file}" ]; then
    local real_file
    real_file="$(readlink -f "${file}" 2>/dev/null || true)"
    if ! is_path_inside_prefix "${real_file}" "${DATA_DIR}"; then
      return 1
    fi
  fi

  # Basic JSON structural check: first non-empty line starts with '{', last with '}'
  local first_char last_char
  first_char="$(sed -n '/^[[:space:]]*$/d; s/^[[:space:]]*\(.\).*/\1/p; q' "${file}")"
  last_char="$(sed -n '/^[[:space:]]*$/d; $ s/.*\(.\)[[:space:]]*$/\1/p' "${file}")"
  if [ "${first_char}" != "{" ] || [ "${last_char}" != "}" ]; then
    return 1
  fi

  local fmt comp ver pref b_dir d_dir bin_p
  fmt="$(extract_json_int "format_version" "${file}")"
  comp="$(extract_json_string "component" "${file}")"
  ver="$(extract_json_string "version" "${file}")"
  pref="$(extract_json_string "prefix" "${file}")"
  b_dir="$(extract_json_string "bin_dir" "${file}")"
  d_dir="$(extract_json_string "data_dir" "${file}")"
  bin_p="$(extract_json_string "binary_path" "${file}")"

  if [ "${fmt}" != "1" ]; then
    return 1
  fi
  if [ "${comp}" != "${COMPONENT_NAME}" ]; then
    return 1
  fi
  if [ -z "${ver}" ] || [ -z "${pref}" ] || [ -z "${b_dir}" ] || [ -z "${d_dir}" ] || [ -z "${bin_p}" ]; then
    return 1
  fi

  # Check owned_files array contains binary_path and manifest
  if ! grep -q "\"owned_files\"" "${file}" 2>/dev/null; then
    return 1
  fi
  if ! grep -q "\"${bin_p}\"" "${file}" 2>/dev/null; then
    return 1
  fi

  return 0
}

# Verify manifest ownership of an existing binary
verify_manifest_ownership() {
  local existing_bin="$1"
  if [ ! -f "${MANIFEST_PATH}" ]; then
    return 1
  fi

  if ! validate_manifest_structure "${MANIFEST_PATH}"; then
    return 1
  fi

  local m_prefix m_bin_dir m_data_dir m_bin
  m_prefix="$(extract_json_string "prefix" "${MANIFEST_PATH}")"
  m_bin_dir="$(extract_json_string "bin_dir" "${MANIFEST_PATH}")"
  m_data_dir="$(extract_json_string "data_dir" "${MANIFEST_PATH}")"
  m_bin="$(extract_json_string "binary_path" "${MANIFEST_PATH}")"

  # Exact canonical path binding verification
  if [ "${m_prefix}" != "${CANONICAL_PREFIX}" ]; then
    return 1
  fi
  if [ "${m_bin_dir}" != "${BIN_DIR}" ]; then
    return 1
  fi
  if [ "${m_data_dir}" != "${DATA_DIR}" ]; then
    return 1
  fi

  # If existing_bin specified, it must match binary_path and be in owned_files
  if [ -n "${existing_bin}" ]; then
    if [ "${m_bin}" != "${existing_bin}" ]; then
      return 1
    fi
    if ! grep -q "\"${existing_bin}\"" "${MANIFEST_PATH}" 2>/dev/null; then
      return 1
    fi

    # Symlink escape check on existing binary
    if [ -L "${existing_bin}" ]; then
      local b_real
      b_real="$(readlink -f "${existing_bin}" 2>/dev/null || true)"
      if ! is_path_inside_prefix "${b_real}" "${BIN_DIR}"; then
        return 1
      fi
    fi
  fi

  return 0
}

# Helper: check if target binary exists
target_binary_exists() {
  [ -f "${TARGET_BIN_PATH}" ] || [ -f "${TARGET_BIN_EXE_PATH}" ]
}

# Helper: get existing target binary path
get_existing_target_binary() {
  if [ -f "${TARGET_BIN_PATH}" ]; then
    echo "${TARGET_BIN_PATH}"
  elif [ -f "${TARGET_BIN_EXE_PATH}" ]; then
    echo "${TARGET_BIN_EXE_PATH}"
  else
    echo ""
  fi
}

# Helper: find built source binary
resolve_source_binary() {
  if [ -n "${SOURCE_BIN_OVERRIDE}" ]; then
    if [ -f "${SOURCE_BIN_OVERRIDE}" ]; then
      echo "${SOURCE_BIN_OVERRIDE}"
      return 0
    else
      echo "Error: Specified --source-bin does not exist: ${SOURCE_BIN_OVERRIDE}" >&2
      exit 1
    fi
  fi

  # Build locally from crate
  if ! command -v cargo >/dev/null 2>&1; then
    echo "Error: 'cargo' is required to build from source but was not found in PATH." >&2
    echo "Please install Rust (https://rustup.rs) or pass a pre-built binary via --source-bin." >&2
    exit 1
  fi

  echo "==> Building dosi-tracker-linux (${CRATE_VERSION}) via cargo --release..." >&2
  cargo build --locked --release --manifest-path "${LINUX_CRATE_DIR}/Cargo.toml" --bin dosi-tracker-linux >&2

  local cand1="${LINUX_CRATE_DIR}/target/release/dosi-tracker-linux"
  local cand2="${LINUX_CRATE_DIR}/target/release/dosi-tracker-linux.exe"
  if [ -f "${cand1}" ]; then
    echo "${cand1}"
  elif [ -f "${cand2}" ]; then
    echo "${cand2}"
  else
    echo "Error: Built binary not found in target/release" >&2
    exit 1
  fi
}

# Subcommand: STATUS
do_status() {
  echo "--- Dosi-Tracker Linux Client Status ---"
  echo "Prefix:   ${CANONICAL_PREFIX}"
  echo "Bin Dir:  ${BIN_DIR}"
  echo "Data Dir: ${DATA_DIR}"

  local cur_bin
  cur_bin="$(get_existing_target_binary)"

  if [ -f "${MANIFEST_PATH}" ] && [ -n "${cur_bin}" ]; then
    if verify_manifest_ownership "${cur_bin}"; then
      echo "Status:   Installed (Managed)"
      if [ -f "${CURRENT_VERSION_PATH}" ]; then
        echo "Version:  $(cat "${CURRENT_VERSION_PATH}")"
      fi
      echo "Binary:   ${cur_bin}"
      echo "Manifest: ${MANIFEST_PATH}"
      echo ""
      echo "Running diagnostic probe (--version):"
      "${cur_bin}" --version || true
    else
      echo "Status:   Corrupted or mismatched manifest at ${MANIFEST_PATH}"
    fi
  elif [ -n "${cur_bin}" ]; then
    echo "Status:   UNMANAGED file present at ${cur_bin} (missing manifest)"
  else
    echo "Status:   Not installed in ${CANONICAL_PREFIX}"
  fi
}

# Subcommand: UNINSTALL
do_uninstall() {
  local cur_bin
  cur_bin="$(get_existing_target_binary)"

  if [ ! -f "${MANIFEST_PATH}" ] && [ -z "${cur_bin}" ]; then
    echo "No Dosi-Tracker installation found in ${CANONICAL_PREFIX}. Nothing to uninstall."
    exit 0
  fi

  if [ -n "${cur_bin}" ] && [ ! -f "${MANIFEST_PATH}" ]; then
    echo "Error: File exists at ${cur_bin} without an ownership manifest." >&2
    echo "Refusing to delete unmanaged or foreign file." >&2
    exit 1
  fi

  if [ -f "${MANIFEST_PATH}" ]; then
    if ! verify_manifest_ownership "${cur_bin}"; then
      echo "Error: Manifest at ${MANIFEST_PATH} is corrupted, invalid, or does not match prefix/binary." >&2
      echo "Refusing to delete files without valid ownership proof." >&2
      exit 1
    fi
  fi

  echo "==> Uninstalling Dosi-Tracker Linux diagnostic client from ${CANONICAL_PREFIX}..."

  # Remove owned binary
  if [ -n "${cur_bin}" ] && [ -f "${cur_bin}" ]; then
    rm -f "${cur_bin}"
    echo "Removed binary: ${cur_bin}"
  fi

  # Remove backups
  rm -f "${BACKUP_BIN_PATH}" "${BACKUP_BIN_EXE_PATH}" "${BACKUP_MANIFEST_PATH}" "${BACKUP_VERSION_PATH}" 2>/dev/null || true
  if [ -d "${BACKUP_DIR}" ]; then
    rmdir "${BACKUP_DIR}" 2>/dev/null || true
  fi

  # Remove current_version
  rm -f "${CURRENT_VERSION_PATH}" 2>/dev/null || true

  # Remove manifest
  rm -f "${MANIFEST_PATH}" 2>/dev/null || true
  echo "Removed manifest: ${MANIFEST_PATH}"

  # Clean data dir if empty
  if [ -d "${DATA_DIR}" ]; then
    rmdir "${DATA_DIR}" 2>/dev/null || true
  fi

  echo "Uninstallation complete."
  echo "User configuration and consent at ~/.config/dosi-tracker were PRESERVED."
}

# Subcommand: ROLLBACK
do_rollback() {
  local backup_bin=""
  if [ -f "${BACKUP_BIN_PATH}" ]; then
    backup_bin="${BACKUP_BIN_PATH}"
  elif [ -f "${BACKUP_BIN_EXE_PATH}" ]; then
    backup_bin="${BACKUP_BIN_EXE_PATH}"
  fi

  if [ -z "${backup_bin}" ] || [ ! -f "${BACKUP_MANIFEST_PATH}" ]; then
    echo "Error: Complete backup (binary and manifest) not found in ${BACKUP_DIR}. Cannot rollback." >&2
    exit 1
  fi

  # Check symlink escapes on backup files
  if [ -L "${backup_bin}" ] || [ -L "${BACKUP_MANIFEST_PATH}" ]; then
    echo "Error: Backup file is a symlink. Rollback aborted." >&2
    exit 1
  fi

  # Validate backup manifest structure
  if ! validate_manifest_structure "${BACKUP_MANIFEST_PATH}"; then
    echo "Error: Backup manifest is corrupted or invalid. Rollback aborted." >&2
    exit 1
  fi

  # Validate current destination binary: if present, must be owned by current manifest
  local cur_bin
  cur_bin="$(get_existing_target_binary)"
  if [ -n "${cur_bin}" ]; then
    if ! verify_manifest_ownership "${cur_bin}"; then
      echo "Error: Existing destination binary '${cur_bin}' is not owned by a valid Dosi-Tracker manifest." >&2
      echo "Refusing to overwrite unmanaged foreign destination during rollback." >&2
      exit 1
    fi
  fi

  local dest_bin="${TARGET_BIN_PATH}"
  if [[ "${backup_bin}" == *.exe.prev ]]; then
    dest_bin="${TARGET_BIN_EXE_PATH}"
  fi

  # Smoke test backup binary before touching destination
  chmod 755 "${backup_bin}" 2>/dev/null || true
  if ! "${backup_bin}" --version >/dev/null 2>&1; then
    echo "Error: Backup binary failed smoke execution check. Rollback aborted." >&2
    exit 1
  fi

  echo "==> Restoring previous installation from ${BACKUP_DIR}..."

  # Stage rollback in temporary staging area
  mkdir -p "${DATA_DIR}"
  STAGING_DIR="${DATA_DIR}/.staging.$$"
  rm -rf "${STAGING_DIR}" 2>/dev/null || true
  mkdir -p "${STAGING_DIR}"

  local staged_bin="${STAGING_DIR}/$(basename "${dest_bin}")"
  local staged_manifest="${STAGING_DIR}/manifest.json"
  local staged_version="${STAGING_DIR}/current_version"

  cp -p "${backup_bin}" "${staged_bin}"
  cp -p "${BACKUP_MANIFEST_PATH}" "${staged_manifest}"
  if [ -f "${BACKUP_VERSION_PATH}" ]; then
    cp -p "${BACKUP_VERSION_PATH}" "${staged_version}"
  else
    local rolled_ver
    rolled_ver="$("${staged_bin}" --version 2>/dev/null | awk '{print $2}' || echo "0.1.0")"
    echo "${rolled_ver}" > "${staged_version}"
  fi

  # Activate restored state
  mv -f "${staged_bin}" "${dest_bin}"
  chmod 755 "${dest_bin}"
  cp -p "${staged_manifest}" "${MANIFEST_PATH}"
  cp -p "${staged_version}" "${CURRENT_VERSION_PATH}"

  rm -rf "${STAGING_DIR}" 2>/dev/null || true
  STAGING_DIR=""

  local final_ver
  final_ver="$(cat "${CURRENT_VERSION_PATH}" 2>/dev/null || echo "unknown")"
  echo "Rollback successful. Restored ${dest_bin} (version ${final_ver})."
}

# Subcommand: INSTALL / UPDATE
do_install_or_update() {
  local is_update="0"
  local existing_bin
  existing_bin="$(get_existing_target_binary)"

  if [ -n "${existing_bin}" ]; then
    if [ ! -f "${MANIFEST_PATH}" ]; then
      echo "Error: Existing file found at ${existing_bin} without an ownership manifest." >&2
      echo "Refusing to overwrite unmanaged or foreign file." >&2
      exit 1
    fi

    if ! verify_manifest_ownership "${existing_bin}"; then
      echo "Error: Manifest at ${MANIFEST_PATH} is invalid or does not match ${existing_bin}." >&2
      echo "Refusing to overwrite unverified file." >&2
      exit 1
    fi
    is_update="1"
  fi

  # 1. Resolve / Build source binary
  local source_bin
  source_bin="$(resolve_source_binary)"

  # 2. Smoke check source binary before touching destination
  chmod 755 "${source_bin}" 2>/dev/null || true
  echo "==> Running pre-install validation on source binary..." >&2
  if ! "${source_bin}" --version >/dev/null 2>&1; then
    echo "Error: Source binary failed execution smoke test ('--version' failed)." >&2
    echo "Installation aborted. Target prefix was NOT modified." >&2
    exit 1
  fi

  local detected_version
  detected_version="$("${source_bin}" --version 2>/dev/null | awk '{print $2}' || echo "${CRATE_VERSION}")"

  local dest_bin_name="${TARGET_BIN_NAME}"
  if [[ "${source_bin}" == *.exe ]]; then
    dest_bin_name="${TARGET_BIN_NAME}.exe"
  fi
  local final_bin="${BIN_DIR}/${dest_bin_name}"

  # 3. Create staging directory inside DATA_DIR
  mkdir -p "${DATA_DIR}"
  STAGING_DIR="${DATA_DIR}/.staging.$$"
  rm -rf "${STAGING_DIR}" 2>/dev/null || true
  mkdir -p "${STAGING_DIR}"

  local staged_bin="${STAGING_DIR}/${dest_bin_name}"
  local staged_manifest="${STAGING_DIR}/manifest.json"
  local staged_version="${STAGING_DIR}/current_version"

  # 4. Copy binary to staging area and verify permissions
  cp -p "${source_bin}" "${staged_bin}"
  chmod 755 "${staged_bin}"

  # 5. Staged Smoke Validation
  if ! "${staged_bin}" --help >/dev/null 2>&1; then
    echo "Error: Staged binary failed invocation test ('--help' failed)." >&2
    echo "Installation aborted. Previous installation was NOT touched." >&2
    exit 1
  fi

  # 6. Generate Staged Manifest & Version metadata
  local prev_version="none"
  if [ -f "${CURRENT_VERSION_PATH}" ]; then
    prev_version="$(cat "${CURRENT_VERSION_PATH}" 2>/dev/null || echo "none")"
  fi

  local timestamp
  timestamp="$(date -u +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || date +"%Y-%m-%d %H:%M:%S")"

  cat > "${staged_manifest}" <<EOF
{
  "format_version": 1,
  "component": "${COMPONENT_NAME}",
  "version": "${detected_version}",
  "installed_at": "${timestamp}",
  "prefix": "${CANONICAL_PREFIX}",
  "bin_dir": "${BIN_DIR}",
  "data_dir": "${DATA_DIR}",
  "binary_path": "${final_bin}",
  "owned_files": [
    "${final_bin}",
    "${MANIFEST_PATH}",
    "${CURRENT_VERSION_PATH}"
  ],
  "previous_version": "${prev_version}"
}
EOF

  echo "${detected_version}" > "${staged_version}"

  # Validate the staged manifest structure before touching live files
  if ! validate_manifest_structure "${staged_manifest}"; then
    echo "Error: Staged manifest failed structural validation." >&2
    exit 1
  fi

  # 7. If updating, back up existing binary AND metadata completely
  local backup_target_bin=""
  local backup_target_manifest=""
  local backup_target_version=""

  if [ "${is_update}" = "1" ] && [ -n "${existing_bin}" ]; then
    mkdir -p "${BACKUP_DIR}"
    backup_target_bin="${BACKUP_DIR}/$(basename "${existing_bin}").prev"
    backup_target_manifest="${BACKUP_MANIFEST_PATH}"
    backup_target_version="${BACKUP_VERSION_PATH}"

    cp -p "${existing_bin}" "${backup_target_bin}"
    cp -p "${MANIFEST_PATH}" "${backup_target_manifest}"
    if [ -f "${CURRENT_VERSION_PATH}" ]; then
      cp -p "${CURRENT_VERSION_PATH}" "${backup_target_version}"
    fi
    echo "==> Backed up existing managed state to ${BACKUP_DIR}" >&2
  fi

  # Helper: Rollback to previous state on activation failure
  rollback_managed_state() {
    if [ "${is_update}" = "1" ]; then
      echo "==> Restoring previous managed state from backup..." >&2
      if [ -n "${backup_target_bin}" ] && [ -f "${backup_target_bin}" ]; then
        cp -p "${backup_target_bin}" "${final_bin}" 2>/dev/null || true
      fi
      if [ -n "${backup_target_manifest}" ] && [ -f "${backup_target_manifest}" ]; then
        cp -p "${backup_target_manifest}" "${MANIFEST_PATH}" 2>/dev/null || true
      fi
      if [ -n "${backup_target_version}" ] && [ -f "${backup_target_version}" ]; then
        cp -p "${backup_target_version}" "${CURRENT_VERSION_PATH}" 2>/dev/null || true
      fi
    else
      echo "==> Cleaning up partial fresh installation..." >&2
      rm -f "${final_bin}" "${MANIFEST_PATH}" "${CURRENT_VERSION_PATH}" 2>/dev/null || true
    fi
  }

  # 8. Activation: Move binary and publish metadata atomically
  mkdir -p "${BIN_DIR}"
  if ! mv -f "${staged_bin}" "${final_bin}"; then
    echo "Error: Failed activating binary into ${final_bin}" >&2
    rollback_managed_state
    exit 1
  fi
  chmod 755 "${final_bin}"

  # Fault injection hook for testing activation metadata failure recovery
  if [ "${DOSI_TEST_INJECT_FAIL_METADATA:-0}" = "1" ]; then
    echo "Injected failure during metadata publication" >&2
    rollback_managed_state
    exit 1
  fi

  if ! cp -p "${staged_manifest}" "${MANIFEST_PATH}"; then
    echo "Error: Failed publishing manifest into ${MANIFEST_PATH}" >&2
    rollback_managed_state
    exit 1
  fi

  if ! cp -p "${staged_version}" "${CURRENT_VERSION_PATH}"; then
    echo "Error: Failed publishing version file into ${CURRENT_VERSION_PATH}" >&2
    rollback_managed_state
    exit 1
  fi

  # Fault injection hook for testing post-activation smoke verification failure recovery
  if [ "${DOSI_TEST_INJECT_FAIL_SMOKE:-0}" = "1" ]; then
    echo "Injected failure during post-activation smoke verification" >&2
    rollback_managed_state
    exit 1
  fi

  # 9. Post-Activation Verification
  echo "==> Verifying activated binary..." >&2
  if ! "${final_bin}" --version >/dev/null 2>&1; then
    echo "Error: Activated binary failed post-install verification!" >&2
    rollback_managed_state
    exit 1
  fi

  # Success: clear staging
  rm -rf "${STAGING_DIR}" 2>/dev/null || true
  STAGING_DIR=""

  echo "--------------------------------------------------------"
  if [ "${is_update}" = "1" ]; then
    echo "Successfully updated Dosi-Tracker Linux client to version ${detected_version}!"
  else
    echo "Successfully installed Dosi-Tracker Linux client version ${detected_version}!"
  fi
  echo "Executable: ${final_bin}"
  echo "Manifest:   ${MANIFEST_PATH}"
  echo ""
  echo "Note: This is the unprivileged, sensor-free diagnostic foundation."
  echo "To verify diagnostic health:"
  echo "    ${final_bin} doctor"
  echo "--------------------------------------------------------"
}

case "${COMMAND}" in
  install)
    do_install_or_update
    ;;
  update)
    do_install_or_update
    ;;
  rollback)
    do_rollback
    ;;
  uninstall)
    do_uninstall
    ;;
  status)
    do_status
    ;;
esac
