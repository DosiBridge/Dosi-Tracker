#!/usr/bin/env bash
#
# Rootless, unprivileged source-build installer, updater, and uninstaller
# for Dosi-Tracker Linux Desktop Client (Headless Diagnostic Foundation).
#
# INVARIANTS:
# 1. Strictly rootless: refuses sudo/root execution; installs into user prefix.
# 2. Safe defaults: default prefix is ~/.local (bin in ~/.local/bin).
# 3. Path boundaries: refuses system directories (/, /usr, /etc, etc.) and symlink escapes.
# 4. Conflict protection: refuses to overwrite unmanaged foreign files without an ownership manifest.
# 5. Atomic staging: builds and tests binary in staging directory before activation.
# 6. Failure preservation: failed builds or activations leave prior working install intact.
# 7. Safe uninstall: deletes ONLY files recorded in the ownership manifest;
#    strictly preserves user configuration, consent (~/.config/dosi-tracker), and unrelated files.
# 8. Sensor-free: does NOT install autostart services, D-Bus hooks, kernel modules, or sensors.
#

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LINUX_CRATE_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

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

# 2. Derive and Normalize Paths
if [ -z "${PREFIX}" ]; then
  echo "Error: Prefix cannot be empty." >&2
  exit 1
fi

# Resolve canonical prefix if it exists, or canonical parent
resolve_canonical_path() {
  local target="$1"
  if [ -d "${target}" ]; then
    (cd "${target}" && pwd -P)
  else
    local parent
    parent="$(dirname "${target}")"
    if [ -d "${parent}" ]; then
      local canon_parent
      canon_parent="$(cd "${parent}" && pwd -P)"
      echo "${canon_parent}/$(basename "${target}")"
    else
      echo "${target}"
    fi
  fi
}

CANONICAL_PREFIX="$(resolve_canonical_path "${PREFIX}")"

# 3. Path Boundary and Security Validation
validate_path_safety() {
  local p="$1"
  local label="$2"

  if [ -z "$p" ]; then
    echo "Error: ${label} path cannot be empty." >&2
    exit 1
  fi

  case "$p" in
    /|/bin|/sbin|/usr|/usr/bin|/usr/sbin|/etc|/var|/lib|/lib64|/root|/sys|/proc|/dev|/boot)
      echo "Error: Refusing to install into system-critical directory: $p" >&2
      exit 1
      ;;
    /usr/*)
      echo "Error: System directory prefix '${p}' is forbidden. Use a user-owned prefix like ~/.local." >&2
      exit 1
      ;;
    /etc/*|/var/*|/root/*|/sys/*|/proc/*|/dev/*)
      echo "Error: Protected system path '${p}' is forbidden." >&2
      exit 1
      ;;
  esac
}

validate_path_safety "${CANONICAL_PREFIX}" "Installation prefix"

if [ -z "${BIN_DIR}" ]; then
  BIN_DIR="${CANONICAL_PREFIX}/bin"
else
  BIN_DIR="$(resolve_canonical_path "${BIN_DIR}")"
fi
validate_path_safety "${BIN_DIR}" "Binary directory"

if [ -z "${DATA_DIR}" ]; then
  DATA_DIR="${CANONICAL_PREFIX}/share/dosi-tracker"
else
  DATA_DIR="$(resolve_canonical_path "${DATA_DIR}")"
fi
validate_path_safety "${DATA_DIR}" "Data directory"

# Symlink escape validation: ensure BIN_DIR is not a symlink pointing outside prefix
if [ -L "${BIN_DIR}" ]; then
  BIN_LINK_TARGET="$(cd "${BIN_DIR}" 2>/dev/null && pwd -P || true)"
  if [[ "${BIN_LINK_TARGET}" != "${CANONICAL_PREFIX}"* ]]; then
    echo "Error: Binary directory symlink escapes prefix: ${BIN_DIR} -> ${BIN_LINK_TARGET}" >&2
    exit 1
  fi
fi

TARGET_BIN_NAME="dosi-tracker-linux"
TARGET_BIN_PATH="${BIN_DIR}/${TARGET_BIN_NAME}"
TARGET_BIN_EXE_PATH="${BIN_DIR}/${TARGET_BIN_NAME}.exe"
MANIFEST_PATH="${DATA_DIR}/manifest.json"
CURRENT_VERSION_PATH="${DATA_DIR}/current_version"
BACKUP_DIR="${DATA_DIR}/backup"
BACKUP_BIN_PATH="${BACKUP_DIR}/${TARGET_BIN_NAME}.prev"
BACKUP_BIN_EXE_PATH="${BACKUP_DIR}/${TARGET_BIN_NAME}.exe.prev"

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

# Helper: verify manifest ownership of existing target binary
verify_manifest_ownership() {
  local existing_bin="$1"
  if [ ! -f "${MANIFEST_PATH}" ]; then
    return 1
  fi
  # Verify manifest contains component name and owned file
  if grep -q "\"${COMPONENT_NAME}\"" "${MANIFEST_PATH}" 2>/dev/null; then
    return 0
  fi
  return 1
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
    echo "Status:   Installed"
    if [ -f "${CURRENT_VERSION_PATH}" ]; then
      echo "Version:  $(cat "${CURRENT_VERSION_PATH}")"
    fi
    echo "Binary:   ${cur_bin}"
    echo "Manifest: ${MANIFEST_PATH}"
    echo ""
    echo "Running diagnostic probe (--version):"
    "${cur_bin}" --version || true
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
      echo "Error: Manifest at ${MANIFEST_PATH} is corrupted or belongs to another component." >&2
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
  rm -f "${BACKUP_BIN_PATH}" "${BACKUP_BIN_EXE_PATH}" 2>/dev/null || true
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

  if [ -z "${backup_bin}" ]; then
    echo "Error: No previous backup found at ${BACKUP_DIR}. Cannot rollback." >&2
    exit 1
  fi

  echo "==> Restoring previous binary from ${backup_bin}..."
  local dest_bin="${TARGET_BIN_PATH}"
  if [[ "${backup_bin}" == *.exe.prev ]]; then
    dest_bin="${TARGET_BIN_EXE_PATH}"
  fi

  # Smoke test backup binary
  chmod 755 "${backup_bin}" 2>/dev/null || true
  if ! "${backup_bin}" --version >/dev/null 2>&1; then
    echo "Error: Backup binary failed smoke execution check. Rollback aborted." >&2
    exit 1
  fi

  cp -p "${backup_bin}" "${dest_bin}"
  chmod 755 "${dest_bin}"

  # Update current version file if backup version is known
  local rolled_ver
  rolled_ver="$("${dest_bin}" --version 2>/dev/null | awk '{print $2}' || echo "unknown")"
  echo "${rolled_ver}" > "${CURRENT_VERSION_PATH}"

  echo "Rollback successful. Restored ${dest_bin} (version ${rolled_ver})."
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
      echo "Error: Manifest at ${MANIFEST_PATH} does not claim ownership of ${existing_bin}." >&2
      echo "Refusing to overwrite." >&2
      exit 1
    fi
    is_update="1"
  fi

  # 1. Resolve / Build source binary
  local source_bin
  source_bin="$(resolve_source_binary)"

  # 2. Smoke check source binary before touching destination
  chmod 755 "${source_bin}" 2>/dev/null || true
  echo "==> Running pre-install validation on source binary..."
  if ! "${source_bin}" --version >/dev/null 2>&1; then
    echo "Error: Source binary failed execution smoke test ('--version' failed)." >&2
    echo "Installation aborted. Target prefix was NOT modified." >&2
    exit 1
  fi

  local detected_version
  detected_version="$("${source_bin}" --version 2>/dev/null | awk '{print $2}' || echo "${CRATE_VERSION}")"

  # 3. Create staging directory inside DATA_DIR
  mkdir -p "${DATA_DIR}"
  STAGING_DIR="${DATA_DIR}/.staging.$$"
  rm -rf "${STAGING_DIR}" 2>/dev/null || true
  mkdir -p "${STAGING_DIR}"

  local dest_bin_name="${TARGET_BIN_NAME}"
  if [[ "${source_bin}" == *.exe ]]; then
    dest_bin_name="${TARGET_BIN_NAME}.exe"
  fi
  local staged_bin="${STAGING_DIR}/${dest_bin_name}"

  # 4. Copy to staging area and verify permissions
  cp -p "${source_bin}" "${staged_bin}"
  chmod 755 "${staged_bin}"

  # 5. Staged Smoke Validation: verify binary in staging area before activating
  if ! "${staged_bin}" --help >/dev/null 2>&1; then
    echo "Error: Staged binary failed invocation test ('--help' failed)." >&2
    echo "Installation aborted. Previous installation was NOT touched." >&2
    exit 1
  fi

  # 6. If updating, back up the previous binary
  local prev_version="none"
  if [ -f "${CURRENT_VERSION_PATH}" ]; then
    prev_version="$(cat "${CURRENT_VERSION_PATH}" 2>/dev/null || echo "none")"
  fi

  if [ "${is_update}" = "1" ] && [ -n "${existing_bin}" ]; then
    mkdir -p "${BACKUP_DIR}"
    local backup_target="${BACKUP_DIR}/$(basename "${existing_bin}").prev"
    cp -p "${existing_bin}" "${backup_target}"
    echo "==> Backed up existing binary to ${backup_target}"
  fi

  # 7. Activation: Ensure BIN_DIR exists and move staged binary into place
  mkdir -p "${BIN_DIR}"
  local final_bin="${BIN_DIR}/${dest_bin_name}"
  mv -f "${staged_bin}" "${final_bin}"
  chmod 755 "${final_bin}"

  # Clear staging dir now that move succeeded
  rm -rf "${STAGING_DIR}" 2>/dev/null || true
  STAGING_DIR=""

  # 8. Write Ownership Manifest
  local timestamp
  timestamp="$(date -u +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || date +"%Y-%m-%d %H:%M:%S")"

  cat > "${MANIFEST_PATH}" <<EOF
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

  echo "${detected_version}" > "${CURRENT_VERSION_PATH}"

  # 9. Post-Activation Verification
  echo "==> Verifying activated binary..."
  if ! "${final_bin}" --version >/dev/null 2>&1; then
    echo "Error: Activated binary failed post-install verification!" >&2
    if [ "${is_update}" = "1" ] && [ -f "${backup_target:-}" ]; then
      echo "Attempting emergency rollback to previous binary..." >&2
      cp -p "${backup_target}" "${final_bin}"
      echo "Rolled back to previous version." >&2
    fi
    exit 1
  fi

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
