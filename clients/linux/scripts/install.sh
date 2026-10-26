#!/usr/bin/env bash
#
# Rootless installer, updater and uninstaller for the Dosi Tracker Linux agent
# (tarball and source installs; .deb/.rpm installs use the package manager).
#
#   ./install.sh [install] [--server URL] [--dashboard URL]   from the tarball
#   scripts/install.sh install --build                         from a checkout
#   install.sh update | rollback | status
#   install.sh uninstall [--purge] [--yes]
#   <prefix>/lib/dosi-tracker/install/uninstall.sh [--purge] [--yes]
#
# STRICT INVARIANTS:
# 1. Strictly rootless: refuses root/sudo; installs into a user prefix (default ~/.local).
# 2. Strict prefix confinement: exact path boundary (no sibling-prefix matches), lexical
#    dot-dot normalization, and every existing ancestor checked against symlink escapes.
# 3. Ownership manifest (format_version 2, <prefix>/lib/dosi-tracker/install/manifest.json):
#    strict structured validation (no duplicate or unknown keys) bound to the canonical
#    prefix, bin_dir and metadata_dir, with owned_files limited to the exact installer
#    inventory. A format_version 1 manifest of the foundation installer
#    (<prefix>/share/dosi-tracker/) is validated the same way and migrated.
# 4. Fail-closed conflict protection: never overwrites or deletes a file that no valid
#    manifest owns, and refuses malformed, mismatched or escaping manifests.
# 5. Full staging: every file is staged and the staged binaries are smoke-tested before
#    anything in the prefix changes; invalid --server/--dashboard URLs exit 2 first.
# 6. Atomic failure recovery: an update stops the agent gracefully, swaps the files,
#    reloads systemd, restarts the agent if it was running and smoke-tests the result;
#    any failure restores every replaced file and the metadata from the backup.
# 7. Rollback validation: the backup manifest and the current ownership are verified
#    before anything is restored.
# 8. Safe uninstall: deletes ONLY manifest-owned files; keeps the configuration and
#    consent (~/.config/dosi-tracker) and the data unless --purge (dosi-tracker reset).
# 9. Never enables anything: no autostart entry is written (dosi-tracker service enable
#    is the user's choice), the user unit is installed but not enabled, no sensors,
#    no D-Bus services, no kernel modules, no network downloads.
#

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
SCRIPT_PATH="${SCRIPT_DIR}/$(basename "${BASH_SOURCE[0]}")"

APP_ID="io.github.dosibridge.DosiTracker"
COMPONENT_NAME="dosi-tracker"
CLI_NAME="dosi-tracker"
GUI_NAME="dosi-tracker-gui"
ICON_SIZES="16x16 22x22 24x24 32x32 48x48 64x64 128x128 256x256 512x512"
METADATA_SUBDIR="lib/dosi-tracker/install"
LEGACY_DATA_SUBDIR="share/dosi-tracker"

# Where this copy runs from:
#   installed  <prefix>/lib/dosi-tracker/install/uninstall.sh (uninstall by default)
#   tarball    install.sh next to bin/dosi-tracker
#   checkout   clients/linux/scripts/install.sh (builds with cargo)
RUN_MODE="checkout"
ASSET_ROOT=""
CRATE_DIR=""
case "${SCRIPT_DIR}" in
  */"${METADATA_SUBDIR}")
    RUN_MODE="installed"
    ;;
  *)
    if [ -f "${SCRIPT_DIR}/bin/${CLI_NAME}" ]; then
      RUN_MODE="tarball"
      ASSET_ROOT="${SCRIPT_DIR}"
    else
      CRATE_DIR="$(cd "${SCRIPT_DIR}/.." && pwd -P)"
      ASSET_ROOT="${CRATE_DIR}"
    fi
    ;;
esac

installer_version() {
  local v=""
  if [ -n "${CRATE_DIR}" ] && [ -f "${CRATE_DIR}/Cargo.toml" ]; then
    v="$(grep '^version' "${CRATE_DIR}/Cargo.toml" | head -n1 | sed -E 's/version[[:space:]]*=[[:space:]]*"([^"]+)"/\1/')"
  elif [ -f "${SCRIPT_DIR}/VERSION" ]; then
    v="$(head -n1 "${SCRIPT_DIR}/VERSION")"
  elif [ -f "${SCRIPT_DIR}/current_version" ]; then
    v="$(head -n1 "${SCRIPT_DIR}/current_version")"
  fi
  printf '%s' "${v:-0.0.0}"
}
CRATE_VERSION="$(installer_version)"

if [ "${RUN_MODE}" = "installed" ]; then
  DEFAULT_PREFIX="$(cd "${SCRIPT_DIR}/../../.." && pwd -P)"
else
  DEFAULT_PREFIX="${HOME}/.local"
fi

PREFIX="${PREFIX:-${DEFAULT_PREFIX}}"
BIN_DIR=""
STAGING_DIR=""
SOURCE_BIN_OVERRIDE="${DOSI_INSTALL_SOURCE_BIN:-}"
SOURCE_GUI_OVERRIDE=""
FORCE_BUILD="0"
SERVER_URL=""
DASHBOARD_URL=""
PURGE="0"
ASSUME_YES="0"
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
Dosi Tracker Linux installer (${CRATE_VERSION})

USAGE:
    install.sh [SUBCOMMAND] [OPTIONS]

SUBCOMMANDS:
    install     Install into the prefix (default action; from the tarball's bin/,
                or built with cargo when run from a source checkout)
    update      Replace an installation: graceful stop, swap, restart, smoke, rollback
    rollback    Restore the installation that the last update replaced
    uninstall   Remove the installed files (keeps your configuration, consent and data)
    status      Show the installation and its manifest

OPTIONS:
    --prefix <DIR>        Rootless installation prefix (default: ~/.local)
    --bin-dir <DIR>       Directory for the programs (default: <PREFIX>/bin)
    --server <URL>        Save the Dosi Tracker server (dosi-tracker config set api_base_url)
    --dashboard <URL>     Save the web dashboard (dosi-tracker config set dashboard_url)
    --build               Build from the source checkout with cargo
    --source-bin <PATH>   Install this dosi-tracker binary (development and tests)
    --source-gui <PATH>   Install this dosi-tracker-gui binary (development and tests)
    --purge               uninstall: also delete your data (dosi-tracker reset)
    --yes                 uninstall --purge: do not ask for confirmation
    --allow-root          Allow running as root (strictly for automated isolated test fixtures)
    -h, --help            Show this help message
    -v, --version         Show installer version

INVARIANTS:
    - Rootless only: never requires or runs with sudo or root privileges.
    - Never enables anything: no autostart entry; the user unit is installed, not enabled.
      Start at login with: dosi-tracker service enable --now
    - Preserves user consent and configuration at ~/.config/dosi-tracker upon uninstall
      (unless --purge).
    - Removes only the files its manifest owns.
EOF
}

usage_error() {
  echo "Error: $*" >&2
  echo "Run with --help for usage." >&2
  exit 2
}

need_value() {
  if [ "$#" -lt 2 ] || [ -z "$2" ]; then
    usage_error "$1 needs a value"
  fi
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    install|update|rollback|uninstall|status)
      if [ -n "${COMMAND}" ]; then
        usage_error "Multiple subcommands specified ('${COMMAND}' and '$1')"
      fi
      COMMAND="$1"
      shift
      ;;
    --prefix) need_value "$@"; PREFIX="$2"; shift 2 ;;
    --prefix=*) PREFIX="${1#*=}"; shift ;;
    --bin-dir) need_value "$@"; BIN_DIR="$2"; shift 2 ;;
    --bin-dir=*) BIN_DIR="${1#*=}"; shift ;;
    --server) need_value "$@"; SERVER_URL="$2"; shift 2 ;;
    --server=*) SERVER_URL="${1#*=}"; shift ;;
    --dashboard) need_value "$@"; DASHBOARD_URL="$2"; shift 2 ;;
    --dashboard=*) DASHBOARD_URL="${1#*=}"; shift ;;
    --source-bin) need_value "$@"; SOURCE_BIN_OVERRIDE="$2"; shift 2 ;;
    --source-bin=*) SOURCE_BIN_OVERRIDE="${1#*=}"; shift ;;
    --source-gui) need_value "$@"; SOURCE_GUI_OVERRIDE="$2"; shift 2 ;;
    --source-gui=*) SOURCE_GUI_OVERRIDE="${1#*=}"; shift ;;
    --build) FORCE_BUILD="1"; shift ;;
    --purge) PURGE="1"; shift ;;
    --yes|-y) ASSUME_YES="1"; shift ;;
    --allow-root) ALLOW_ROOT="1"; shift ;;
    -h|--help)
      usage
      exit 0
      ;;
    -v|--version)
      echo "dosi-tracker installer version ${CRATE_VERSION}"
      exit 0
      ;;
    *)
      usage_error "Unknown argument '$1'."
      ;;
  esac
done

if [ -z "${COMMAND}" ]; then
  if [ "${RUN_MODE}" = "installed" ]; then
    COMMAND="uninstall"
  else
    COMMAND="install"
  fi
fi

if { [ "${PURGE}" = "1" ] || [ "${ASSUME_YES}" = "1" ]; } && [ "${COMMAND}" != "uninstall" ]; then
  usage_error "--purge and --yes only apply to uninstall"
fi
if { [ -n "${SERVER_URL}" ] || [ -n "${DASHBOARD_URL}" ]; } && [ "${COMMAND}" != "install" ] && [ "${COMMAND}" != "update" ]; then
  usage_error "--server and --dashboard only apply to install and update"
fi

# 1. Root / sudo check (before anything else touches the file system)
if [ "${ALLOW_ROOT}" != "1" ]; then
  EUID_VAL="${EUID:-$(id -u 2>/dev/null || echo 1000)}"
  USER_VAL="${USER:-$(whoami 2>/dev/null || echo unknown)}"
  if [ "${EUID_VAL}" -eq 0 ] || [ "${USER_VAL}" = "root" ]; then
    echo "Error: Refusing to run as root/sudo." >&2
    echo "Dosi Tracker is installed per user and never runs as root: run this as the user who will be tracked." >&2
    echo "System-wide installs use the .deb or .rpm package." >&2
    exit 1
  fi
fi

# 2. Path helpers and prefix confinement

# Normalize a path lexically (handles relative components and dot-dot without resolving symlinks)
normalize_path_lexical() {
  local raw="$1"
  if [ -z "$raw" ]; then
    echo ""
    return 0
  fi
  if [[ "$raw" != /* ]]; then
    raw="$(pwd -P)/${raw}"
  fi
  raw="$(printf '%s' "${raw}" | sed -E 's|/+|/|g')"

  local IFS='/'
  local -a segments
  read -ra segments <<< "${raw}"
  local stack=()
  local seg
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

# Resolve symlinks in the longest existing ancestor; keep the missing tail as is.
canonicalize_existing_prefix() {
  local norm="$1"
  local head="${norm}" tail=""
  while [ ! -d "${head}" ]; do
    tail="/$(basename "${head}")${tail}"
    head="$(dirname "${head}")"
  done
  local resolved
  resolved="$(cd -P "${head}" 2>/dev/null && pwd -P || printf '%s' "${head}")"
  if [ "${resolved}" = "/" ]; then
    resolved=""
  fi
  printf '%s' "${resolved}${tail}"
}

# Strict boundary check: 0 if target is inside or equal to prefix (separator boundary)
is_path_inside_prefix() {
  local target="$1"
  local prefix="$2"
  if [ -z "$target" ] || [ -z "$prefix" ]; then
    return 1
  fi
  local clean_target="${target%/}"
  local clean_prefix="${prefix%/}"
  if [ "${clean_target}" = "${clean_prefix}" ]; then
    return 0
  fi
  if [[ "${clean_target}" == "${clean_prefix}/"* ]]; then
    return 0
  fi
  return 1
}

refuse_system_path() {
  local path="$1" label="$2"
  case "$path" in
    /|/bin|/sbin|/usr|/usr/bin|/usr/sbin|/etc|/var|/lib|/lib64|/root|/sys|/proc|/dev|/boot|/opt)
      echo "Error: Refusing to use system-critical directory '${path}' as ${label}." >&2
      exit 1
      ;;
    /usr/*|/etc/*|/var/*|/root/*|/sys/*|/proc/*|/dev/*|/boot/*)
      echo "Error: Protected system path '${path}' is forbidden as ${label}. Use a user-owned prefix like ~/.local." >&2
      exit 1
      ;;
  esac
}

# Verify path safety, blacklist and strict prefix confinement
verify_path_confinement() {
  local target="$1"
  local expected_prefix="$2"
  local label="$3"

  local norm_target
  norm_target="$(normalize_path_lexical "$target")"
  refuse_system_path "${norm_target}" "${label}"

  if ! is_path_inside_prefix "$norm_target" "$expected_prefix"; then
    echo "Error: ${label} '${target}' (normalized: '${norm_target}') is outside selected prefix '${expected_prefix}'." >&2
    exit 1
  fi

  # Walk down the components to detect ancestor symlink escapes before any directory is created
  local current=""
  local IFS='/'
  local -a segments
  read -ra segments <<< "${norm_target}"
  local seg
  for seg in "${segments[@]}"; do
    if [ -z "$seg" ]; then
      continue
    fi
    current="${current}/${seg}"
    if [ -L "$current" ]; then
      local link_dest
      link_dest="$(cd -P "$current" 2>/dev/null && pwd -P || readlink -f "$current" 2>/dev/null || true)"
      if [ -z "$link_dest" ] || ! is_path_inside_prefix "$link_dest" "$expected_prefix"; then
        echo "Error: Symlink escape detected in ${label}: '${current}' points outside prefix to '${link_dest:-?}'." >&2
        exit 1
      fi
    fi
  done
}

if [ -z "${PREFIX}" ]; then
  echo "Error: Prefix cannot be empty." >&2
  exit 1
fi

NORM_PREFIX="$(normalize_path_lexical "${PREFIX}")"
refuse_system_path "${NORM_PREFIX}" "installation prefix"
CANONICAL_PREFIX="$(canonicalize_existing_prefix "${NORM_PREFIX}")"
refuse_system_path "${CANONICAL_PREFIX}" "installation prefix"

if [ -z "${BIN_DIR}" ]; then
  RAW_BIN_DIR="${CANONICAL_PREFIX}/bin"
else
  RAW_BIN_DIR="${BIN_DIR}"
fi
verify_path_confinement "${RAW_BIN_DIR}" "${CANONICAL_PREFIX}" "Binary directory"
BIN_DIR="$(canonicalize_existing_prefix "$(normalize_path_lexical "${RAW_BIN_DIR}")")"
if ! is_path_inside_prefix "${BIN_DIR}" "${CANONICAL_PREFIX}"; then
  echo "Error: Resolved binary directory '${BIN_DIR}' escapes prefix '${CANONICAL_PREFIX}'." >&2
  exit 1
fi

META_DIR="${CANONICAL_PREFIX}/${METADATA_SUBDIR}"
verify_path_confinement "${META_DIR}" "${CANONICAL_PREFIX}" "Metadata directory"
LEGACY_DATA_DIR="${CANONICAL_PREFIX}/${LEGACY_DATA_SUBDIR}"
SHARE_DIR="${CANONICAL_PREFIX}/share"

TARGET_BIN_PATH="${BIN_DIR}/${CLI_NAME}"
TARGET_GUI_PATH="${BIN_DIR}/${GUI_NAME}"
MANIFEST_PATH="${META_DIR}/manifest.json"
CURRENT_VERSION_PATH="${META_DIR}/current_version"
UNINSTALLER_PATH="${META_DIR}/uninstall.sh"
BACKUP_DIR="${META_DIR}/backup"
BACKUP_MANIFEST_PATH="${BACKUP_DIR}/manifest.json.prev"
BACKUP_VERSION_PATH="${BACKUP_DIR}/current_version.prev"
BACKUP_FILES_DIR="${BACKUP_DIR}/files"
LEGACY_MANIFEST_PATH="${LEGACY_DATA_DIR}/manifest.json"
LEGACY_VERSION_PATH="${LEGACY_DATA_DIR}/current_version"
LEGACY_BACKUP_DIR="${LEGACY_DATA_DIR}/backup"

DESKTOP_PATH="${SHARE_DIR}/applications/${APP_ID}.desktop"
UNIT_PATH="${SHARE_DIR}/systemd/user/dosi-tracker.service"
BASH_COMPLETION_PATH="${SHARE_DIR}/bash-completion/completions/${CLI_NAME}"
ZSH_COMPLETION_PATH="${SHARE_DIR}/zsh/site-functions/_${CLI_NAME}"
FISH_COMPLETION_PATH="${SHARE_DIR}/fish/vendor_completions.d/${CLI_NAME}.fish"

# Every file this installer may ever own in this prefix (one per line).
inventory_paths() {
  printf '%s\n' "${TARGET_BIN_PATH}" "${TARGET_GUI_PATH}" "${DESKTOP_PATH}"
  local size
  for size in ${ICON_SIZES}; do
    printf '%s\n' "${SHARE_DIR}/icons/hicolor/${size}/apps/${APP_ID}.png"
  done
  printf '%s\n' "${SHARE_DIR}/icons/hicolor/scalable/apps/${APP_ID}.svg"
  printf '%s\n' "${UNIT_PATH}" "${BASH_COMPLETION_PATH}" "${ZSH_COMPLETION_PATH}" "${FISH_COMPLETION_PATH}"
  printf '%s\n' "${MANIFEST_PATH}" "${CURRENT_VERSION_PATH}" "${UNINSTALLER_PATH}"
}

is_inventory_path() {
  local candidate="$1" known
  while IFS= read -r known; do
    if [ "${candidate}" = "${known}" ]; then
      return 0
    fi
  done < <(inventory_paths)
  return 1
}

is_metadata_path() {
  case "$1" in
    "${MANIFEST_PATH}"|"${CURRENT_VERSION_PATH}"|"${UNINSTALLER_PATH}") return 0 ;;
  esac
  return 1
}

# 3. Structured JSON manifest validation (no eval/source, strict destination binding)

find_python_prerequisite() {
  if command -v python3 >/dev/null 2>&1; then
    echo "python3"
  elif command -v python >/dev/null 2>&1 \
    && python -c "import sys; sys.exit(0 if sys.version_info.major >= 3 else 1)" >/dev/null 2>&1; then
    echo "python"
  else
    return 1
  fi
}

require_python() {
  find_python_prerequisite >/dev/null || {
    echo "Error: Python 3 is required to validate the installation manifest but was not found in PATH." >&2
    echo "Install python3 with your package manager and run the installer again." >&2
    exit 1
  }
}

# Format 2 manifest at <prefix>/lib/dosi-tracker/install, bound to this prefix.
validate_manifest_v2() {
  local manifest_path="$1"
  local py_bin
  py_bin="$(find_python_prerequisite)" || {
    echo "Error: Python 3 is required for structured JSON manifest validation but was not found in PATH." >&2
    return 1
  }
  if [ ! -f "${manifest_path}" ] || [ -L "${manifest_path}" ]; then
    return 1
  fi

  {
    printf '%s\n' "${CANONICAL_PREFIX}" "${BIN_DIR}" "${META_DIR}" "${COMPONENT_NAME}" "${APP_ID}" "${ICON_SIZES}"
    cat "${manifest_path}"
  } | "${py_bin}" -c '
import json
import os
import posixpath
import sys

exp_prefix, exp_bin_dir, exp_meta_dir, exp_component, app_id, icon_sizes = (
    sys.stdin.readline().rstrip("\r\n") for _ in range(6)
)
raw_manifest = sys.stdin.read()


def fail(message):
    sys.stderr.write("Error: " + message + "\n")
    sys.exit(1)


def no_duplicate_keys(pairs):
    seen = {}
    for key, value in pairs:
        if key in seen:
            raise ValueError("Duplicate key found in manifest: %r" % key)
        seen[key] = value
    return seen


try:
    data = json.loads(raw_manifest, object_pairs_hook=no_duplicate_keys)
except ValueError as e:
    fail("Manifest is invalid: %s" % e)

if not isinstance(data, dict):
    fail("Manifest root must be a JSON object")

allowed_keys = {
    "format_version", "component", "version", "installed_at", "source", "prefix",
    "bin_dir", "metadata_dir", "binary_path", "owned_files", "previous_version",
}
unexpected = set(data) - allowed_keys
if unexpected:
    fail("Unexpected keys in manifest: %r" % sorted(unexpected))
missing = allowed_keys - set(data)
if missing:
    fail("Missing keys in manifest: %r" % sorted(missing))

fmt = data["format_version"]
if type(fmt) is not int or fmt != 2:
    fail("format_version must be integer 2, got %r" % (fmt,))
if data["component"] != exp_component:
    fail("component must be %r, got %r" % (exp_component, data["component"]))
for key in ("version", "installed_at"):
    value = data[key]
    if not isinstance(value, str) or not value.strip():
        fail("%s must be a non-empty string" % key)
if data["source"] not in ("tarball", "build", "binary"):
    fail("source must be tarball, build or binary, got %r" % (data["source"],))
for key, expected in (("prefix", exp_prefix), ("bin_dir", exp_bin_dir), ("metadata_dir", exp_meta_dir)):
    if data[key] != expected:
        fail("%s mismatch: expected %r, got %r" % (key, expected, data[key]))
binary_path = exp_bin_dir + "/dosi-tracker"
if data["binary_path"] != binary_path:
    fail("binary_path mismatch: expected %r, got %r" % (binary_path, data["binary_path"]))

share = exp_prefix + "/share"
inventory = {
    binary_path,
    exp_bin_dir + "/dosi-tracker-gui",
    share + "/applications/" + app_id + ".desktop",
    share + "/icons/hicolor/scalable/apps/" + app_id + ".svg",
    share + "/systemd/user/dosi-tracker.service",
    share + "/bash-completion/completions/dosi-tracker",
    share + "/zsh/site-functions/_dosi-tracker",
    share + "/fish/vendor_completions.d/dosi-tracker.fish",
    exp_meta_dir + "/manifest.json",
    exp_meta_dir + "/current_version",
    exp_meta_dir + "/uninstall.sh",
}
for size in icon_sizes.split():
    inventory.add(share + "/icons/hicolor/" + size + "/apps/" + app_id + ".png")

owned = data["owned_files"]
if not isinstance(owned, list) or not owned:
    fail("owned_files must be a non-empty array")

boundary = exp_prefix.rstrip("/") + "/"
seen = set()
for item in owned:
    if not isinstance(item, str):
        fail("Entry in owned_files is not a string: %r" % (item,))
    if any(ord(c) < 32 or ord(c) == 127 for c in item):
        fail("Control character detected in owned_files entry: %r" % item)
    if item != item.strip():
        fail("Leading or trailing whitespace in owned_files entry: %r" % item)
    if not item.startswith("/"):
        fail("owned_files entry must be an absolute path: %r" % item)
    if item.endswith("/") or "//" in item:
        fail("Non-canonical path in owned_files entry: %r" % item)
    if any(part in (".", "..") for part in item.split("/")):
        fail("Traversal or dot segment in owned_files entry: %r" % item)
    if not item.startswith(boundary):
        fail("owned file %r is outside prefix %r" % (item, exp_prefix))
    if item not in inventory:
        fail("Entry in owned_files is not an allowed installer-managed file: %r" % item)
    parent = posixpath.dirname(item)
    while parent and parent != exp_prefix and parent.startswith(boundary):
        if os.path.islink(parent):
            resolved = os.path.realpath(parent)
            if not (resolved == exp_prefix or resolved.startswith(boundary)):
                fail("Parent directory of owned file is a symlink escaping prefix: %r -> %r" % (parent, resolved))
        parent = posixpath.dirname(parent)
    if item in seen:
        fail("Duplicate file in owned_files array: %r" % item)
    seen.add(item)

for required in (binary_path, exp_meta_dir + "/manifest.json", exp_meta_dir + "/current_version"):
    if required not in seen:
        fail("%r is not contained in owned_files" % required)

previous = data["previous_version"]
if previous is not None and (not isinstance(previous, str) or not previous.strip()):
    fail("previous_version must be null or a non-empty string, got %r" % (previous,))
sys.exit(0)
'
}

# Format 1 manifest of the foundation installer (<prefix>/share/dosi-tracker), bound to
# this prefix; only migrated, never written.
validate_manifest_v1() {
  local manifest_path="$1"
  local exp_bin_path="$2"
  local py_bin
  py_bin="$(find_python_prerequisite)" || {
    echo "Error: Python 3 is required for structured JSON manifest validation but was not found in PATH." >&2
    return 1
  }
  if [ ! -f "${manifest_path}" ]; then
    return 1
  fi
  if [ -L "${manifest_path}" ]; then
    local real_file
    real_file="$(readlink -f "${manifest_path}" 2>/dev/null || true)"
    if ! is_path_inside_prefix "${real_file}" "${LEGACY_DATA_DIR}"; then
      echo "Error: Manifest file '${manifest_path}' is a symlink escaping data dir '${LEGACY_DATA_DIR}'." >&2
      return 1
    fi
  fi

  {
    printf '%s\n' "${CANONICAL_PREFIX}" "${BIN_DIR}" "${LEGACY_DATA_DIR}" "${exp_bin_path}"
    cat "${manifest_path}"
  } | "${py_bin}" -c '
import json
import os
import posixpath
import sys

exp_prefix, exp_bin_dir, exp_data_dir, exp_bin_path = (
    sys.stdin.readline().rstrip("\r\n") for _ in range(4)
)
raw_manifest = sys.stdin.read()


def fail(message):
    sys.stderr.write("Error: " + message + "\n")
    sys.exit(1)


def no_duplicate_keys(pairs):
    seen = {}
    for key, value in pairs:
        if key in seen:
            raise ValueError("Duplicate key found in manifest: %r" % key)
        seen[key] = value
    return seen


try:
    data = json.loads(raw_manifest, object_pairs_hook=no_duplicate_keys)
except ValueError as e:
    fail("Manifest is invalid: %s" % e)

if not isinstance(data, dict):
    fail("Manifest root must be a JSON object")

allowed_keys = {
    "format_version", "component", "version", "installed_at",
    "prefix", "bin_dir", "data_dir", "binary_path", "owned_files", "previous_version",
}
unexpected = set(data) - allowed_keys
if unexpected:
    fail("Unexpected keys in manifest: %r" % sorted(unexpected))

fmt = data.get("format_version")
if type(fmt) is not int or fmt != 1:
    fail("format_version must be integer 1, got %r" % (fmt,))
if data.get("component") not in ("dosi-tracker-linux-diagnostic-foundation", "dosi-tracker-linux"):
    fail("component mismatch: %r" % (data.get("component"),))
for key in ("version", "installed_at"):
    value = data.get(key)
    if not isinstance(value, str) or not value.strip():
        fail("%s must be a non-empty string" % key)
for key, expected in (("prefix", exp_prefix), ("bin_dir", exp_bin_dir), ("data_dir", exp_data_dir)):
    if data.get(key) != expected:
        fail("%s mismatch: expected %r, got %r" % (key, expected, data.get(key)))

bin_p = data.get("binary_path")
candidates = {exp_bin_path, exp_bin_path + ".exe"}
if bin_p not in candidates:
    fail("binary_path mismatch: expected %r, got %r" % (exp_bin_path, bin_p))

owned = data.get("owned_files")
if not isinstance(owned, list) or not owned:
    fail("owned_files must be a non-empty array")
allowed_owned = candidates | {exp_data_dir + "/manifest.json", exp_data_dir + "/current_version"}
boundary = exp_prefix.rstrip("/") + "/"
seen = set()
for item in owned:
    if not isinstance(item, str):
        fail("Entry in owned_files is not a string: %r" % (item,))
    if any(ord(c) < 32 or ord(c) == 127 for c in item):
        fail("Control character detected in owned_files entry: %r" % item)
    if item != item.strip():
        fail("Leading or trailing whitespace in owned_files entry: %r" % item)
    if not item.startswith("/") or item.endswith("/") or "//" in item:
        fail("Non-canonical path in owned_files entry: %r" % item)
    if any(part in (".", "..") for part in item.split("/")):
        fail("Traversal or dot segment in owned_files entry: %r" % item)
    if not item.startswith(boundary):
        fail("owned file %r is outside prefix %r" % (item, exp_prefix))
    if item not in allowed_owned:
        fail("Entry in owned_files is not an allowed installer-managed file: %r" % item)
    parent = posixpath.dirname(item)
    while parent and parent != exp_prefix and parent.startswith(boundary):
        if os.path.islink(parent):
            resolved = os.path.realpath(parent)
            if not (resolved == exp_prefix or resolved.startswith(boundary)):
                fail("Parent directory of owned file is a symlink escaping prefix: %r -> %r" % (parent, resolved))
        parent = posixpath.dirname(parent)
    if item in seen:
        fail("Duplicate file in owned_files array: %r" % item)
    seen.add(item)
if bin_p not in seen:
    fail("binary_path %r is not contained in owned_files array" % bin_p)
previous = data.get("previous_version")
if previous is not None and (not isinstance(previous, str) or not previous.strip()):
    fail("previous_version must be null or non-empty string, got %r" % (previous,))
sys.exit(0)
'
}

# One field of a manifest (already validated)
get_manifest_field() {
  local manifest_file="$1"
  local field_name="$2"
  local py_bin
  py_bin="$(find_python_prerequisite)" || return 1
  {
    printf '%s\n' "${field_name}"
    cat "${manifest_file}"
  } | "${py_bin}" -c '
import json
import sys

field = sys.stdin.readline().rstrip("\r\n")
try:
    value = json.load(sys.stdin).get(field)
except Exception:
    sys.exit(1)
if value is not None:
    print(json.dumps(value) if isinstance(value, (dict, list)) else value)
'
}

# owned_files of a manifest as a NUL-delimited stream
get_manifest_owned_files() {
  local manifest_file="$1"
  local py_bin
  py_bin="$(find_python_prerequisite)" || return 1
  "${py_bin}" -c '
import json
import sys

try:
    for item in json.load(sys.stdin).get("owned_files", []):
        if isinstance(item, str) and item:
            sys.stdout.buffer.write(item.encode("utf-8") + b"\0")
except Exception:
    sys.exit(1)
' < "${manifest_file}"
}

# Write a format 2 manifest: fixed fields, then the owned files (one per line) on stdin.
generate_manifest_v2() {
  local out_file="$1" version="$2" installed_at="$3" source_kind="$4" prev_version="$5"
  local py_bin
  py_bin="$(find_python_prerequisite)" || {
    echo "Error: Python 3 is required for JSON manifest generation." >&2
    return 1
  }
  {
    printf '%s\n' "${COMPONENT_NAME}" "${version}" "${installed_at}" "${source_kind}" \
      "${CANONICAL_PREFIX}" "${BIN_DIR}" "${META_DIR}" "${TARGET_BIN_PATH}" "${prev_version}"
    cat
  } | "${py_bin}" -c '
import json
import sys

fields = [sys.stdin.readline().rstrip("\r\n") for _ in range(9)]
component, version, installed_at, source, prefix, bin_dir, meta_dir, binary_path, previous = fields
owned = [line.rstrip("\r\n") for line in sys.stdin if line.strip()]
for name, value in zip(
    ("component", "version", "installed_at", "source", "prefix", "bin_dir", "metadata_dir", "binary_path"),
    fields,
):
    if any(ord(c) < 32 or ord(c) == 127 for c in value):
        sys.stderr.write("Error: Invalid control character in %s\n" % name)
        sys.exit(1)
data = {
    "format_version": 2,
    "component": component,
    "version": version,
    "installed_at": installed_at,
    "source": source,
    "prefix": prefix,
    "bin_dir": bin_dir,
    "metadata_dir": meta_dir,
    "binary_path": binary_path,
    "owned_files": owned,
    "previous_version": previous if previous not in ("", "none", "null") else None,
}
sys.stdout.write(json.dumps(data, indent=2) + "\n")
' > "${out_file}"
}

# Render the desktop entry template for this bin dir: Exec arguments quoted as the
# Desktop Entry Specification requires (the same rules as service::exec_quote).
render_desktop_entry() {
  local template="$1" out_file="$2"
  local py_bin
  py_bin="$(find_python_prerequisite)" || return 1
  {
    printf '%s\n' "${BIN_DIR}"
    cat "${template}"
  } | "${py_bin}" -c '
import sys

bindir = sys.stdin.readline().rstrip("\r\n")
RESERVED = set(" \t\n\"\x27\\><~|&;$*?#()`")


def exec_quote(arg):
    arg = arg.replace("%", "%%")
    if not any(c in RESERVED for c in arg):
        return arg
    out = "\""
    for c in arg:
        if c in "\"`$\\":
            out += "\\"
        out += c
    return out + "\""


def desktop_string(value):
    return value.replace("\\", "\\\\")


for line in sys.stdin:
    line = line.rstrip("\r\n")
    if line.startswith("Exec=@BINDIR@/"):
        program, _, rest = line[len("Exec=@BINDIR@/"):].partition(" ")
        line = "Exec=" + desktop_string(exec_quote(bindir + "/" + program)) + ((" " + rest) if rest else "")
    elif line.startswith("TryExec=@BINDIR@/"):
        line = "TryExec=" + desktop_string(bindir + "/" + line[len("TryExec=@BINDIR@/"):])
    if "@BINDIR@" in line:
        sys.stderr.write("Error: unrendered placeholder in the desktop entry: %s\n" % line)
        sys.exit(1)
    sys.stdout.write(line + "\n")
' > "${out_file}"
}

# The systemd user unit for this bin dir, or nothing (exit 1) when the directory has
# characters a unit cannot use as is (the same rule as service::render_unit).
render_user_unit() {
  local template="$1" out_file="$2"
  case "${BIN_DIR}" in
    *[[:space:]]*|*\"*|*\'*|*\\*|*%*|*\$*|*\;*) return 1 ;;
  esac
  sed "s|@BINDIR@|${BIN_DIR}|g" "${template}" > "${out_file}"
  ! grep -q '@BINDIR@' "${out_file}"
}

# Is the prefix ~/.local, whose share/systemd/user is in the user manager's search path?
prefix_is_user_local() {
  local home_local
  home_local="$(canonicalize_existing_prefix "$(normalize_path_lexical "${HOME}/.local")")"
  [ "${CANONICAL_PREFIX}" = "${home_local}" ]
}

# 4. Installation state

existing_binary() {
  if [ -f "${TARGET_BIN_PATH}" ] || [ -L "${TARGET_BIN_PATH}" ]; then
    echo "${TARGET_BIN_PATH}"
  elif [ -f "${TARGET_BIN_PATH}.exe" ]; then
    echo "${TARGET_BIN_PATH}.exe"
  fi
}

# Sets INSTALL_STATE (none | v2 | v1) or exits when the prefix holds files that no valid
# manifest owns.
detect_install_state() {
  local action="$1"
  local cur_bin
  cur_bin="$(existing_binary)"
  INSTALL_STATE="none"
  if [ -e "${MANIFEST_PATH}" ] || [ -L "${MANIFEST_PATH}" ]; then
    require_python
    if ! validate_manifest_v2 "${MANIFEST_PATH}"; then
      echo "Error: Manifest at ${MANIFEST_PATH} is corrupted, invalid, or does not match this prefix." >&2
      echo "Refusing to ${action} without valid ownership proof." >&2
      exit 1
    fi
    if [ -n "${cur_bin}" ] && [ -L "${cur_bin}" ]; then
      echo "Error: ${cur_bin} is a symlink; refusing to ${action}." >&2
      exit 1
    fi
    INSTALL_STATE="v2"
    return 0
  fi
  if [ -e "${LEGACY_MANIFEST_PATH}" ] || [ -L "${LEGACY_MANIFEST_PATH}" ]; then
    require_python
    if ! validate_manifest_v1 "${LEGACY_MANIFEST_PATH}" "${TARGET_BIN_PATH}"; then
      echo "Error: Manifest at ${LEGACY_MANIFEST_PATH} is corrupted, invalid, or does not match this prefix." >&2
      echo "Refusing to ${action} without valid ownership proof." >&2
      exit 1
    fi
    if [ -n "${cur_bin}" ] && [ -L "${cur_bin}" ]; then
      echo "Error: ${cur_bin} is a symlink; refusing to ${action}." >&2
      exit 1
    fi
    INSTALL_STATE="v1"
    return 0
  fi
  if [ -n "${cur_bin}" ]; then
    echo "Error: ${cur_bin} exists without an ownership manifest." >&2
    echo "Refusing to ${action} an unmanaged or foreign file." >&2
    exit 1
  fi
}

# Owned files of the current installation (one per line, NUL-free paths).
current_owned_files() {
  local manifest=""
  case "${INSTALL_STATE}" in
    v2) manifest="${MANIFEST_PATH}" ;;
    v1) manifest="${LEGACY_MANIFEST_PATH}" ;;
    *) return 0 ;;
  esac
  local item
  while IFS= read -r -d '' item; do
    printf '%s\n' "${item}"
  done < <(get_manifest_owned_files "${manifest}")
}

# Independent bash guard for every path the installer deletes or overwrites.
guard_owned_path() {
  local path="$1" allow_legacy="${2:-0}"
  if [[ "${path}" == *$'\n'* ]] || [[ "${path}" == *$'\r'* ]] || [[ "${path}" == *".."* ]]; then
    echo "Error: Refusing to touch non-canonical or traversal path '${path}'." >&2
    exit 1
  fi
  if ! is_path_inside_prefix "${path}" "${CANONICAL_PREFIX}"; then
    echo "Error: Manifest entry '${path}' escapes canonical prefix '${CANONICAL_PREFIX}'." >&2
    exit 1
  fi
  if is_inventory_path "${path}"; then
    return 0
  fi
  if [ "${allow_legacy}" = "1" ]; then
    case "${path}" in
      "${TARGET_BIN_PATH}.exe"|"${LEGACY_MANIFEST_PATH}"|"${LEGACY_VERSION_PATH}") return 0 ;;
    esac
  fi
  echo "Error: Refusing to touch unexpected file '${path}' not in the installer inventory." >&2
  exit 1
}

# Run the installed (or candidate) CLI quietly; never fails the caller.
run_cli_quiet() {
  local bin="$1"
  shift
  [ -x "${bin}" ] || return 1
  "${bin}" "$@" >/dev/null 2>&1
}

# Pending + parked blocks of the signed-in account, or empty when unknown.
unsent_block_count() {
  local bin="$1"
  [ -x "${bin}" ] || return 0
  local py_bin json
  py_bin="$(find_python_prerequisite)" || return 0
  json="$("${bin}" --json queue status 2>/dev/null)" || true
  [ -n "${json}" ] || return 0
  printf '%s' "${json}" | "${py_bin}" -c '
import json
import sys

try:
    result = json.load(sys.stdin).get("result") or {}
    count = int(result.get("pending") or 0) + int(result.get("parked") or 0)
except Exception:
    sys.exit(0)
print(count)
' 2>/dev/null || true
}

agent_is_running() {
  run_cli_quiet "$1" service status
}

reload_user_manager() {
  if command -v systemctl >/dev/null 2>&1; then
    systemctl --user daemon-reload >/dev/null 2>&1 || true
  fi
}

# Refresh desktop and icon caches that already exist in this prefix (never creates one).
refresh_desktop_caches() {
  if [ -f "${SHARE_DIR}/applications/mimeinfo.cache" ] && command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database -q "${SHARE_DIR}/applications" >/dev/null 2>&1 || true
  fi
  if [ -f "${SHARE_DIR}/icons/hicolor/icon-theme.cache" ] && command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -q -t -f "${SHARE_DIR}/icons/hicolor" >/dev/null 2>&1 || true
  fi
}

# Validate a URL with the candidate binary's own validator, against a throw-away copy
# of the user's configuration (so the real one is untouched); exit 2 when invalid.
validate_url_setting() {
  local bin="$1" key="$2" value="$3"
  [ -n "${value}" ] || return 0
  local scratch code=0
  scratch="$(mktemp -d)"
  mkdir -p "${scratch}/config/dosi-tracker"
  chmod 700 "${scratch}" "${scratch}/config" "${scratch}/config/dosi-tracker"
  local user_config="${XDG_CONFIG_HOME:-${HOME}/.config}/dosi-tracker/config.toml"
  if [ -f "${user_config}" ] && [ ! -L "${user_config}" ]; then
    cp "${user_config}" "${scratch}/config/dosi-tracker/config.toml"
    chmod 600 "${scratch}/config/dosi-tracker/config.toml"
  fi
  env -u XDG_RUNTIME_DIR -u DOSI_SOCKET XDG_CONFIG_HOME="${scratch}/config" \
    XDG_DATA_HOME="${scratch}/data" XDG_STATE_HOME="${scratch}/state" \
    "${bin}" --no-start config set "${key}" "${value}" >/dev/null 2>"${scratch}/err" || code=$?
  if [ "${code}" -ne 0 ]; then
    echo "Error: ${key} '${value}' was refused:" >&2
    sed 's/^/    /' "${scratch}/err" >&2 || true
    rm -rf "${scratch}"
    if [ "${code}" -eq 2 ]; then
      echo "Nothing was installed or changed." >&2
      exit 2
    fi
    echo "Error: the configuration check failed (exit ${code}); nothing was installed or changed." >&2
    exit 1
  fi
  rm -rf "${scratch}"
}

# 5. Sources

# Sets SOURCE_BIN, SOURCE_GUI (may be empty) and SOURCE_KIND.
resolve_sources() {
  SOURCE_GUI=""
  if [ -n "${SOURCE_BIN_OVERRIDE}" ]; then
    if [ ! -f "${SOURCE_BIN_OVERRIDE}" ]; then
      echo "Error: Specified --source-bin does not exist: ${SOURCE_BIN_OVERRIDE}" >&2
      exit 1
    fi
    SOURCE_BIN="${SOURCE_BIN_OVERRIDE}"
    SOURCE_KIND="binary"
    if [ -n "${SOURCE_GUI_OVERRIDE}" ]; then
      if [ ! -f "${SOURCE_GUI_OVERRIDE}" ]; then
        echo "Error: Specified --source-gui does not exist: ${SOURCE_GUI_OVERRIDE}" >&2
        exit 1
      fi
      SOURCE_GUI="${SOURCE_GUI_OVERRIDE}"
    fi
    return 0
  fi
  if [ -n "${SOURCE_GUI_OVERRIDE}" ]; then
    usage_error "--source-gui needs --source-bin"
  fi

  if [ "${RUN_MODE}" = "tarball" ] && [ "${FORCE_BUILD}" != "1" ]; then
    SOURCE_BIN="${ASSET_ROOT}/bin/${CLI_NAME}"
    if [ -f "${ASSET_ROOT}/bin/${GUI_NAME}" ]; then
      SOURCE_GUI="${ASSET_ROOT}/bin/${GUI_NAME}"
    fi
    SOURCE_KIND="tarball"
    return 0
  fi

  if [ "${RUN_MODE}" != "checkout" ] || [ ! -f "${CRATE_DIR}/Cargo.toml" ]; then
    echo "Error: --build needs the source checkout (clients/linux/scripts/install.sh)." >&2
    exit 1
  fi
  if ! command -v cargo >/dev/null 2>&1; then
    echo "Error: 'cargo' is required to build from source but was not found in PATH." >&2
    echo "Install Rust (https://rustup.rs), or use the release tarball." >&2
    exit 1
  fi
  echo "==> Building dosi-tracker (${CRATE_VERSION}) with cargo --release..." >&2
  cargo build --locked --release --manifest-path "${CRATE_DIR}/Cargo.toml" --bins >&2

  # Cargo honours CARGO_TARGET_DIR (CI keeps its build in a separate volume).
  local target_dir="${CARGO_TARGET_DIR:-${CRATE_DIR}/target}"
  if [ ! -f "${target_dir}/release/${CLI_NAME}" ]; then
    echo "Error: Built binary not found in ${target_dir}/release" >&2
    exit 1
  fi
  SOURCE_BIN="${target_dir}/release/${CLI_NAME}"
  if [ -f "${target_dir}/release/${GUI_NAME}" ]; then
    SOURCE_GUI="${target_dir}/release/${GUI_NAME}"
  fi
  SOURCE_KIND="build"
}

asset_path() {
  local relative="$1"
  if [ -n "${ASSET_ROOT}" ] && [ -f "${ASSET_ROOT}/packaging/${relative}" ]; then
    printf '%s' "${ASSET_ROOT}/packaging/${relative}"
  fi
}

# Stage one payload file: staging/root/<path relative to the prefix>.
stage_file() {
  local source="$1" final="$2" mode="$3"
  local staged="${STAGING_DIR}/root/${final#"${CANONICAL_PREFIX}"/}"
  mkdir -p "$(dirname "${staged}")"
  cp "${source}" "${staged}"
  chmod "${mode}" "${staged}"
  STAGED_FINALS+=("${final}")
}

# 6. Subcommands

do_status() {
  echo "--- Dosi Tracker Linux installation ---"
  echo "Prefix:    ${CANONICAL_PREFIX}"
  echo "Bin Dir:   ${BIN_DIR}"
  echo "Metadata:  ${META_DIR}"

  local cur_bin
  cur_bin="$(existing_binary)"
  if [ -f "${MANIFEST_PATH}" ]; then
    if [ -n "${cur_bin}" ] && validate_manifest_v2 "${MANIFEST_PATH}"; then
      echo "Status:    Installed (managed, manifest format 2)"
      if [ -f "${CURRENT_VERSION_PATH}" ]; then
        echo "Version:   $(cat "${CURRENT_VERSION_PATH}")"
      fi
      echo "Source:    $(get_manifest_field "${MANIFEST_PATH}" source)"
      echo "Binary:    ${cur_bin}"
      echo "Manifest:  ${MANIFEST_PATH}"
      echo ""
      "${cur_bin}" --version || true
    else
      echo "Status:    Corrupted or mismatched manifest at ${MANIFEST_PATH}"
    fi
  elif [ -f "${LEGACY_MANIFEST_PATH}" ]; then
    echo "Status:    Installed by the foundation installer (manifest format 1 at ${LEGACY_MANIFEST_PATH}); 'install.sh update' migrates it"
  elif [ -n "${cur_bin}" ]; then
    echo "Status:    UNMANAGED file present at ${cur_bin} (missing manifest)"
  else
    echo "Status:    Not installed in ${CANONICAL_PREFIX}"
  fi
}

confirm_purge() {
  local count="$1"
  if [ "${ASSUME_YES}" = "1" ]; then
    return 0
  fi
  if [ ! -t 0 ] || [ ! -r /dev/tty ]; then
    echo "Error: --purge deletes your Dosi Tracker data (${count:-an unknown number of} blocks not uploaded); add --yes to confirm." >&2
    exit 2
  fi
  local answer=""
  printf 'Delete your Dosi Tracker configuration, consent, queue (%s blocks not uploaded) and logs? [y/N] ' "${count:-?}" >/dev/tty
  read -r answer </dev/tty || true
  case "${answer}" in
    y|Y|yes|YES) return 0 ;;
  esac
  echo "Nothing was removed." >&2
  exit 1
}

remove_empty_metadata_dirs() {
  rmdir "${BACKUP_FILES_DIR}" 2>/dev/null || true
  rmdir "${BACKUP_DIR}" 2>/dev/null || true
  rmdir "${META_DIR}" 2>/dev/null || true
  rmdir "${CANONICAL_PREFIX}/lib/dosi-tracker" 2>/dev/null || true
}

remove_legacy_metadata() {
  if [ -d "${LEGACY_BACKUP_DIR}" ] && [ ! -L "${LEGACY_BACKUP_DIR}" ]; then
    rm -f "${LEGACY_BACKUP_DIR}/dosi-tracker.prev" "${LEGACY_BACKUP_DIR}/dosi-tracker.exe.prev" \
      "${LEGACY_BACKUP_DIR}/manifest.json.prev" "${LEGACY_BACKUP_DIR}/current_version.prev" 2>/dev/null || true
    rmdir "${LEGACY_BACKUP_DIR}" 2>/dev/null || true
  elif [ -L "${LEGACY_BACKUP_DIR}" ]; then
    rm -f "${LEGACY_BACKUP_DIR}" 2>/dev/null || true
  fi
  rm -f "${LEGACY_MANIFEST_PATH}" "${LEGACY_VERSION_PATH}" 2>/dev/null || true
  if [ -d "${LEGACY_DATA_DIR}" ] && [ ! -L "${LEGACY_DATA_DIR}" ]; then
    rmdir "${LEGACY_DATA_DIR}" 2>/dev/null || true
  fi
}

do_uninstall() {
  if [ "${PURGE}" = "1" ] && [ "${ASSUME_YES}" != "1" ] && { [ ! -t 0 ] || [ ! -r /dev/tty ]; }; then
    echo "Error: --purge deletes your Dosi Tracker data; add --yes to confirm (no terminal to ask on)." >&2
    exit 2
  fi
  detect_install_state "delete files"
  if [ "${INSTALL_STATE}" = "none" ]; then
    echo "No Dosi Tracker installation found in ${CANONICAL_PREFIX}. Nothing to uninstall."
    exit 0
  fi

  # Validate every entry before the first deletion.
  local -a owned=()
  local item
  while IFS= read -r item; do
    [ -n "${item}" ] || continue
    if [ "${INSTALL_STATE}" = "v1" ]; then
      guard_owned_path "${item}" 1
    else
      guard_owned_path "${item}"
    fi
    owned+=("${item}")
  done < <(current_owned_files)

  local cur_bin unsent
  cur_bin="$(existing_binary)"
  unsent="$(unsent_block_count "${cur_bin}")"

  if [ "${PURGE}" = "1" ]; then
    if [ -z "${cur_bin}" ]; then
      echo "Error: ${TARGET_BIN_PATH} is missing, so your data cannot be purged; nothing was uninstalled." >&2
      exit 1
    fi
    confirm_purge "${unsent}"
    echo "==> Deleting your Dosi Tracker data (dosi-tracker reset)..."
    if ! "${cur_bin}" reset --yes; then
      echo "Error: 'dosi-tracker reset' failed; nothing was uninstalled." >&2
      exit 1
    fi
    # The agent must not keep running from deleted files, and the managed
    # login entry must not keep pointing at a program that is about to be
    # removed, exactly as the non-purge branch below ensures.
    run_cli_quiet "${cur_bin}" service disable || true
  else
    # The agent must not keep running from deleted files; stop it gracefully and
    # remove the managed login entry that would start a program that is gone.
    if agent_is_running "${cur_bin}"; then
      echo "==> Stopping the Dosi Tracker agent..."
      run_cli_quiet "${cur_bin}" service stop || echo "Warning: the agent did not stop cleanly." >&2
    fi
    run_cli_quiet "${cur_bin}" service disable || true
  fi

  echo "==> Uninstalling Dosi Tracker from ${CANONICAL_PREFIX}..."
  local unit_removed="0"
  for item in "${owned[@]}"; do
    if is_metadata_path "${item}"; then
      continue
    fi
    if [ -f "${item}" ] || [ -L "${item}" ]; then
      rm -f "${item}"
      echo "Removed owned file: ${item}"
      if [ "${item}" = "${UNIT_PATH}" ]; then
        unit_removed="1"
      fi
    fi
  done

  if [ -d "${BACKUP_DIR}" ] && [ ! -L "${BACKUP_DIR}" ]; then
    rm -rf "${BACKUP_FILES_DIR}" 2>/dev/null || true
    rm -f "${BACKUP_MANIFEST_PATH}" "${BACKUP_VERSION_PATH}" "${BACKUP_DIR}/uninstall.sh.prev" 2>/dev/null || true
  elif [ -L "${BACKUP_DIR}" ]; then
    rm -f "${BACKUP_DIR}" 2>/dev/null || true
  fi
  rm -f "${CURRENT_VERSION_PATH}" "${MANIFEST_PATH}" "${UNINSTALLER_PATH}" 2>/dev/null || true
  remove_empty_metadata_dirs
  remove_legacy_metadata
  echo "Removed manifest: ${MANIFEST_PATH}"

  if [ "${unit_removed}" = "1" ]; then
    reload_user_manager
  fi
  refresh_desktop_caches

  echo "Uninstallation complete."
  if [ "${PURGE}" = "1" ]; then
    echo "Your Dosi Tracker configuration, consent and data were deleted."
  else
    echo "User configuration and consent at ~/.config/dosi-tracker were PRESERVED."
    if [ -n "${unsent}" ] && [ "${unsent}" != "0" ]; then
      echo "${unsent} blocks that were not uploaded are kept in ~/.local/share/dosi-tracker;"
      echo "they upload after a reinstall, or delete them with 'dosi-tracker reset' before uninstalling."
    fi
  fi
}

# Restore the files recorded in the backup; files of the current inventory that the
# backup does not own are removed.
do_rollback() {
  if [ ! -f "${BACKUP_MANIFEST_PATH}" ] || [ ! -f "${BACKUP_FILES_DIR}/${TARGET_BIN_PATH#"${CANONICAL_PREFIX}"/}" ]; then
    echo "Error: Complete backup (binary and manifest) not found in ${BACKUP_DIR}. Cannot rollback." >&2
    exit 1
  fi
  if [ -L "${BACKUP_DIR}" ] || [ -L "${BACKUP_MANIFEST_PATH}" ] || [ -L "${BACKUP_FILES_DIR}" ]; then
    echo "Error: Backup is a symlink. Rollback aborted." >&2
    exit 1
  fi
  require_python
  if ! validate_manifest_v2 "${BACKUP_MANIFEST_PATH}"; then
    echo "Error: Backup manifest is corrupted, invalid, or does not match this prefix. Rollback aborted." >&2
    exit 1
  fi

  # The current destination must be owned by the current manifest.
  if [ -n "$(existing_binary)" ]; then
    if [ ! -f "${MANIFEST_PATH}" ] || ! validate_manifest_v2 "${MANIFEST_PATH}"; then
      echo "Error: Existing destination binary is not owned by a valid Dosi Tracker manifest." >&2
      echo "Refusing to overwrite an unmanaged foreign destination during rollback." >&2
      exit 1
    fi
  fi

  local -a restore=() current=()
  local item
  while IFS= read -r -d '' item; do
    guard_owned_path "${item}"
    is_metadata_path "${item}" || restore+=("${item}")
  done < <(get_manifest_owned_files "${BACKUP_MANIFEST_PATH}")
  if [ -f "${MANIFEST_PATH}" ]; then
    while IFS= read -r -d '' item; do
      guard_owned_path "${item}"
      is_metadata_path "${item}" || current+=("${item}")
    done < <(get_manifest_owned_files "${MANIFEST_PATH}")
  fi

  local backup_copy
  for item in "${restore[@]}"; do
    backup_copy="${BACKUP_FILES_DIR}/${item#"${CANONICAL_PREFIX}"/}"
    if [ ! -f "${backup_copy}" ] || [ -L "${backup_copy}" ]; then
      echo "Error: The backup lacks ${item}. Rollback aborted." >&2
      exit 1
    fi
  done

  local backup_bin="${BACKUP_FILES_DIR}/${TARGET_BIN_PATH#"${CANONICAL_PREFIX}"/}"
  if ! "${backup_bin}" --version >/dev/null 2>&1; then
    echo "Error: Backup binary failed smoke execution check. Rollback aborted." >&2
    exit 1
  fi

  echo "==> Restoring the previous installation from ${BACKUP_DIR}..."
  local cur_bin was_running="0"
  cur_bin="$(existing_binary)"
  if [ -n "${cur_bin}" ] && agent_is_running "${cur_bin}"; then
    was_running="1"
    run_cli_quiet "${cur_bin}" service stop || true
  fi

  for item in "${restore[@]}"; do
    mkdir -p "$(dirname "${item}")"
    cp -p "${BACKUP_FILES_DIR}/${item#"${CANONICAL_PREFIX}"/}" "${item}.rollback.$$"
    mv -f "${item}.rollback.$$" "${item}"
  done
  local keep
  for item in "${current[@]}"; do
    keep="0"
    for backup_item in "${restore[@]}"; do
      if [ "${item}" = "${backup_item}" ]; then
        keep="1"
        break
      fi
    done
    if [ "${keep}" = "0" ]; then
      rm -f "${item}"
    fi
  done
  cp -p "${BACKUP_MANIFEST_PATH}" "${MANIFEST_PATH}"
  if [ -f "${BACKUP_VERSION_PATH}" ]; then
    cp -p "${BACKUP_VERSION_PATH}" "${CURRENT_VERSION_PATH}"
  else
    get_manifest_field "${BACKUP_MANIFEST_PATH}" version > "${CURRENT_VERSION_PATH}"
  fi

  reload_user_manager
  refresh_desktop_caches
  if [ "${was_running}" = "1" ]; then
    run_cli_quiet "${TARGET_BIN_PATH}" service start || echo "Warning: the agent did not start again; run 'dosi-tracker service start'." >&2
  fi
  echo "Rollback successful. Restored ${TARGET_BIN_PATH} (version $(cat "${CURRENT_VERSION_PATH}"))."
}

do_install_or_update() {
  if [ "${RUN_MODE}" = "installed" ]; then
    echo "Error: this is the installed uninstaller; run install.sh from the release tarball to install or update." >&2
    exit 2
  fi
  require_python
  detect_install_state "overwrite files"
  local is_update="0" migrate_v1="0"
  case "${INSTALL_STATE}" in
    v2) is_update="1" ;;
    v1) is_update="1"; migrate_v1="1" ;;
  esac

  local -a old_owned=()
  local item
  while IFS= read -r item; do
    [ -n "${item}" ] || continue
    if [ "${migrate_v1}" = "1" ]; then
      guard_owned_path "${item}" 1
    else
      guard_owned_path "${item}"
    fi
    old_owned+=("${item}")
  done < <(current_owned_files)

  # 1. Sources
  resolve_sources

  # 2. Smoke-check the sources before anything in the prefix changes
  echo "==> Running pre-install validation on the source binaries..." >&2
  if ! "${SOURCE_BIN}" --version >/dev/null 2>&1; then
    echo "Error: Source binary failed execution smoke test ('--version' failed)." >&2
    echo "Installation aborted. Target prefix was NOT modified." >&2
    exit 1
  fi
  if [ -n "${SOURCE_GUI}" ] && ! "${SOURCE_GUI}" --version >/dev/null 2>&1; then
    echo "Error: Source GUI binary failed execution smoke test ('--version' failed)." >&2
    echo "Installation aborted. Target prefix was NOT modified." >&2
    exit 1
  fi
  local detected_version
  detected_version="$("${SOURCE_BIN}" --version 2>/dev/null | awk '{print $2}')"
  detected_version="${detected_version:-${CRATE_VERSION}}"

  # 3. Settings are validated by the new binary itself (exit 2 before any change)
  validate_url_setting "${SOURCE_BIN}" api_base_url "${SERVER_URL}"
  validate_url_setting "${SOURCE_BIN}" dashboard_url "${DASHBOARD_URL}"

  # 4. Stage every file
  mkdir -p "${META_DIR}"
  STAGING_DIR="${META_DIR}/.staging.$$"
  rm -rf "${STAGING_DIR}" 2>/dev/null || true
  mkdir -p "${STAGING_DIR}/root"
  STAGED_FINALS=()

  stage_file "${SOURCE_BIN}" "${TARGET_BIN_PATH}" 755
  if [ -n "${SOURCE_GUI}" ]; then
    stage_file "${SOURCE_GUI}" "${TARGET_GUI_PATH}" 755
  fi

  local template rendered size
  template="$(asset_path "desktop/${APP_ID}.desktop")"
  if [ -n "${SOURCE_GUI}" ] && [ -n "${template}" ]; then
    rendered="${STAGING_DIR}/${APP_ID}.desktop"
    render_desktop_entry "${template}" "${rendered}"
    stage_file "${rendered}" "${DESKTOP_PATH}" 644
  fi
  for size in ${ICON_SIZES}; do
    template="$(asset_path "icons/hicolor/${size}/apps/${APP_ID}.png")"
    if [ -n "${template}" ]; then
      stage_file "${template}" "${SHARE_DIR}/icons/hicolor/${size}/apps/${APP_ID}.png" 644
    fi
  done
  template="$(asset_path "icons/hicolor/scalable/apps/${APP_ID}.svg")"
  if [ -n "${template}" ]; then
    stage_file "${template}" "${SHARE_DIR}/icons/hicolor/scalable/apps/${APP_ID}.svg" 644
  fi

  template="$(asset_path "systemd/dosi-tracker.service.in")"
  local unit_note=""
  if [ -n "${template}" ]; then
    if ! prefix_is_user_local; then
      unit_note="The prefix is not ~/.local, so no user unit was installed; 'dosi-tracker service enable' writes one to ~/.config/systemd/user."
    elif render_user_unit "${template}" "${STAGING_DIR}/dosi-tracker.service"; then
      stage_file "${STAGING_DIR}/dosi-tracker.service" "${UNIT_PATH}" 644
    else
      unit_note="'${BIN_DIR}' contains characters a systemd unit cannot use, so no user unit was installed."
    fi
  fi

  local shell completion_path
  for shell in bash zsh fish; do
    case "${shell}" in
      bash) completion_path="${BASH_COMPLETION_PATH}" ;;
      zsh) completion_path="${ZSH_COMPLETION_PATH}" ;;
      fish) completion_path="${FISH_COMPLETION_PATH}" ;;
    esac
    if "${SOURCE_BIN}" completions "${shell}" > "${STAGING_DIR}/completion.${shell}" 2>/dev/null \
      && [ -s "${STAGING_DIR}/completion.${shell}" ]; then
      stage_file "${STAGING_DIR}/completion.${shell}" "${completion_path}" 644
    fi
  done

  # Every staged destination must be free or ours.
  local final owned_item ours
  for final in "${STAGED_FINALS[@]}"; do
    guard_owned_path "${final}"
    if [ -e "${final}" ] || [ -L "${final}" ]; then
      ours="0"
      for owned_item in "${old_owned[@]}"; do
        if [ "${owned_item}" = "${final}" ]; then
          ours="1"
          break
        fi
      done
      if [ "${ours}" != "1" ] || [ -L "${final}" ]; then
        echo "Error: ${final} exists and is not owned by this installation." >&2
        echo "Refusing to overwrite an unmanaged or foreign file. Installation aborted; nothing was changed." >&2
        exit 1
      fi
    fi
  done

  # Staged metadata: the uninstaller (this script), the manifest and the version
  cp "${SCRIPT_PATH}" "${STAGING_DIR}/uninstall.sh"
  chmod 755 "${STAGING_DIR}/uninstall.sh"
  local prev_version="none"
  if [ "${INSTALL_STATE}" = "v2" ] && [ -f "${CURRENT_VERSION_PATH}" ]; then
    prev_version="$(head -n1 "${CURRENT_VERSION_PATH}" 2>/dev/null || echo none)"
  elif [ "${migrate_v1}" = "1" ] && [ -f "${LEGACY_VERSION_PATH}" ]; then
    prev_version="$(head -n1 "${LEGACY_VERSION_PATH}" 2>/dev/null || echo none)"
  fi
  local timestamp
  timestamp="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
  printf '%s\n' "${STAGED_FINALS[@]}" "${MANIFEST_PATH}" "${CURRENT_VERSION_PATH}" "${UNINSTALLER_PATH}" \
    | generate_manifest_v2 "${STAGING_DIR}/manifest.json" "${detected_version}" "${timestamp}" "${SOURCE_KIND}" "${prev_version}"
  echo "${detected_version}" > "${STAGING_DIR}/current_version"
  if ! validate_manifest_v2 "${STAGING_DIR}/manifest.json"; then
    echo "Error: Staged manifest failed structured validation." >&2
    exit 1
  fi

  # 5. Staged smoke validation
  if ! "${STAGING_DIR}/root/${TARGET_BIN_PATH#"${CANONICAL_PREFIX}"/}" --help >/dev/null 2>&1; then
    echo "Error: Staged binary failed invocation test ('--help' failed)." >&2
    echo "Installation aborted. Previous installation was NOT touched." >&2
    exit 1
  fi

  # 6. Back up everything this run may replace or remove
  local -a touched=()
  local seen_item
  for item in "${STAGED_FINALS[@]}" "${old_owned[@]}"; do
    is_metadata_path "${item}" && continue
    case "${item}" in
      "${LEGACY_MANIFEST_PATH}"|"${LEGACY_VERSION_PATH}") continue ;;
    esac
    ours="0"
    for seen_item in "${touched[@]}"; do
      if [ "${seen_item}" = "${item}" ]; then
        ours="1"
        break
      fi
    done
    [ "${ours}" = "1" ] || touched+=("${item}")
  done

  local backup_new="${META_DIR}/.backup.$$"
  rm -rf "${backup_new}"
  mkdir -p "${backup_new}/files"
  local -a existed=()
  for item in "${touched[@]}"; do
    if [ -f "${item}" ]; then
      mkdir -p "$(dirname "${backup_new}/files/${item#"${CANONICAL_PREFIX}"/}")"
      cp -p "${item}" "${backup_new}/files/${item#"${CANONICAL_PREFIX}"/}"
      existed+=("${item}")
    fi
  done
  local had_metadata="0"
  if [ "${INSTALL_STATE}" = "v2" ]; then
    had_metadata="1"
    cp -p "${MANIFEST_PATH}" "${backup_new}/manifest.json.prev"
    [ -f "${CURRENT_VERSION_PATH}" ] && cp -p "${CURRENT_VERSION_PATH}" "${backup_new}/current_version.prev"
    [ -f "${UNINSTALLER_PATH}" ] && cp -p "${UNINSTALLER_PATH}" "${backup_new}/uninstall.sh.prev"
  elif [ "${migrate_v1}" = "1" ]; then
    # The foundation install as a format 2 manifest, so 'rollback' can restore it.
    local legacy_version
    legacy_version="$(get_manifest_field "${LEGACY_MANIFEST_PATH}" version)"
    printf '%s\n' "${TARGET_BIN_PATH}" "${MANIFEST_PATH}" "${CURRENT_VERSION_PATH}" \
      | generate_manifest_v2 "${backup_new}/manifest.json.prev" "${legacy_version}" \
        "$(get_manifest_field "${LEGACY_MANIFEST_PATH}" installed_at)" binary none
    printf '%s\n' "${legacy_version}" > "${backup_new}/current_version.prev"
    if ! validate_manifest_v2 "${backup_new}/manifest.json.prev"; then
      echo "Error: the foundation install could not be described for rollback. Nothing was changed." >&2
      rm -rf "${backup_new}"
      exit 1
    fi
  fi

  # 7. Stop the agent gracefully before its files change
  local old_bin was_running="0"
  old_bin="$(existing_binary)"
  if [ "${is_update}" = "1" ] && [ -n "${old_bin}" ] && agent_is_running "${old_bin}"; then
    echo "==> Stopping the Dosi Tracker agent (its block is saved first)..." >&2
    if ! "${old_bin}" service stop >/dev/null; then
      echo "Error: the running agent did not stop; nothing was changed." >&2
      rm -rf "${backup_new}"
      exit 1
    fi
    was_running="1"
  fi

  if [ "${is_update}" = "1" ]; then
    rm -rf "${BACKUP_DIR}"
    mv "${backup_new}" "${BACKUP_DIR}"
    echo "==> Backed up the existing installation to ${BACKUP_DIR}" >&2
  else
    rm -rf "${backup_new}"
  fi

  # Restore the state before this run (used on any failure from here on).
  rollback_managed_state() {
    echo "==> Restoring the previous installation..." >&2
    local path existed_item was_there
    for path in "${touched[@]}"; do
      was_there="0"
      for existed_item in "${existed[@]}"; do
        if [ "${existed_item}" = "${path}" ]; then
          was_there="1"
          break
        fi
      done
      if [ "${was_there}" = "1" ] && [ -f "${BACKUP_DIR}/files/${path#"${CANONICAL_PREFIX}"/}" ]; then
        mkdir -p "$(dirname "${path}")"
        cp -p "${BACKUP_DIR}/files/${path#"${CANONICAL_PREFIX}"/}" "${path}" 2>/dev/null || true
      else
        rm -f "${path}" 2>/dev/null || true
      fi
    done
    if [ "${had_metadata}" = "1" ]; then
      cp -p "${BACKUP_DIR}/manifest.json.prev" "${MANIFEST_PATH}" 2>/dev/null || true
      [ -f "${BACKUP_DIR}/current_version.prev" ] && cp -p "${BACKUP_DIR}/current_version.prev" "${CURRENT_VERSION_PATH}" 2>/dev/null
      [ -f "${BACKUP_DIR}/uninstall.sh.prev" ] && cp -p "${BACKUP_DIR}/uninstall.sh.prev" "${UNINSTALLER_PATH}" 2>/dev/null
    else
      rm -f "${MANIFEST_PATH}" "${CURRENT_VERSION_PATH}" "${UNINSTALLER_PATH}" 2>/dev/null || true
      if [ "${is_update}" != "1" ]; then
        remove_empty_metadata_dirs
      fi
    fi
    reload_user_manager
    if [ "${was_running}" = "1" ] && [ -n "$(existing_binary)" ]; then
      run_cli_quiet "$(existing_binary)" service start || true
    fi
  }

  # 8. Activation: payload files, then the metadata
  for final in "${STAGED_FINALS[@]}"; do
    mkdir -p "$(dirname "${final}")"
    if ! mv -f "${STAGING_DIR}/root/${final#"${CANONICAL_PREFIX}"/}" "${final}"; then
      echo "Error: Failed activating ${final}" >&2
      rollback_managed_state
      exit 1
    fi
  done
  local stale keep_it
  for stale in "${old_owned[@]}"; do
    is_metadata_path "${stale}" && continue
    case "${stale}" in
      "${LEGACY_MANIFEST_PATH}"|"${LEGACY_VERSION_PATH}") continue ;;
    esac
    keep_it="0"
    for final in "${STAGED_FINALS[@]}"; do
      if [ "${final}" = "${stale}" ]; then
        keep_it="1"
        break
      fi
    done
    [ "${keep_it}" = "1" ] || rm -f "${stale}"
  done

  # Fault injection hook for testing activation metadata failure recovery
  if [ "${DOSI_TEST_INJECT_FAIL_METADATA:-0}" = "1" ]; then
    echo "Injected failure during metadata publication" >&2
    rollback_managed_state
    exit 1
  fi

  if ! cp -p "${STAGING_DIR}/uninstall.sh" "${UNINSTALLER_PATH}" \
    || ! cp -p "${STAGING_DIR}/manifest.json" "${MANIFEST_PATH}" \
    || ! cp -p "${STAGING_DIR}/current_version" "${CURRENT_VERSION_PATH}"; then
    echo "Error: Failed publishing the installation metadata into ${META_DIR}" >&2
    rollback_managed_state
    exit 1
  fi

  # 9. systemd and desktop caches, then the settings
  reload_user_manager
  refresh_desktop_caches
  local setting
  for setting in "api_base_url=${SERVER_URL}" "dashboard_url=${DASHBOARD_URL}"; do
    if [ -n "${setting#*=}" ] && ! "${TARGET_BIN_PATH}" --no-start config set "${setting%%=*}" "${setting#*=}" >/dev/null; then
      echo "Error: saving ${setting%%=*} failed." >&2
      rollback_managed_state
      exit 1
    fi
  done

  # Fault injection hook for testing post-activation smoke verification failure recovery
  if [ "${DOSI_TEST_INJECT_FAIL_SMOKE:-0}" = "1" ]; then
    echo "Injected failure during post-activation smoke verification" >&2
    rollback_managed_state
    exit 1
  fi

  # 10. Post-activation smoke: the programs, then the agent if it was running
  echo "==> Verifying the activated installation..." >&2
  if ! "${TARGET_BIN_PATH}" --version >/dev/null 2>&1 \
    || { [ -n "${SOURCE_GUI}" ] && ! "${TARGET_GUI_PATH}" --version >/dev/null 2>&1; }; then
    echo "Error: Activated binary failed post-install verification!" >&2
    rollback_managed_state
    exit 1
  fi
  if [ "${was_running}" = "1" ]; then
    echo "==> Starting the agent again..." >&2
    if ! "${TARGET_BIN_PATH}" service start >/dev/null; then
      echo "Error: the updated agent did not start." >&2
      rollback_managed_state
      exit 1
    fi
  fi

  if [ "${migrate_v1}" = "1" ]; then
    remove_legacy_metadata
    echo "==> Migrated the foundation installer's manifest to ${MANIFEST_PATH}" >&2
  fi

  rm -rf "${STAGING_DIR}" 2>/dev/null || true
  STAGING_DIR=""

  echo "--------------------------------------------------------"
  if [ "${is_update}" = "1" ]; then
    echo "Successfully updated Dosi Tracker to version ${detected_version}."
  else
    echo "Successfully installed Dosi Tracker version ${detected_version}."
  fi
  echo "Programs:  ${TARGET_BIN_PATH}$([ -n "${SOURCE_GUI}" ] && printf ' %s' "${TARGET_GUI_PATH}")"
  echo "Manifest:  ${MANIFEST_PATH}"
  echo "Uninstall: ${UNINSTALLER_PATH} [--purge] [--yes]"
  if [ -n "${unit_note}" ]; then
    echo "Note: ${unit_note}"
  fi
  case ":${PATH}:" in
    *":${BIN_DIR}:"*) ;;
    *) echo "Note: ${BIN_DIR} is not in your PATH; add it, or run ${TARGET_BIN_PATH} directly." ;;
  esac
  echo ""
  if [ "${is_update}" != "1" ]; then
    echo "Nothing starts by itself. Next steps:"
    echo "    dosi-tracker service enable --now     # start the agent now and at every login"
    echo "    dosi-tracker login                    # sign in and choose your workspace"
    echo "    dosi-tracker consent show             # what may be recorded; then consent grant ..."
    echo "    dosi-tracker doctor                   # check this computer"
  fi
  echo "--------------------------------------------------------"
}

case "${COMMAND}" in
  install|update)
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
