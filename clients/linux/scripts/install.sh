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
      echo "dosi-tracker installer version ${CRATE_VERSION}"
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

TARGET_BIN_NAME="dosi-tracker"
TARGET_BIN_PATH="${BIN_DIR}/${TARGET_BIN_NAME}"
TARGET_BIN_EXE_PATH="${BIN_DIR}/${TARGET_BIN_NAME}.exe"
MANIFEST_PATH="${DATA_DIR}/manifest.json"
CURRENT_VERSION_PATH="${DATA_DIR}/current_version"
BACKUP_DIR="${DATA_DIR}/backup"
BACKUP_BIN_PATH="${BACKUP_DIR}/${TARGET_BIN_NAME}.prev"
BACKUP_BIN_EXE_PATH="${BACKUP_DIR}/${TARGET_BIN_NAME}.exe.prev"
BACKUP_MANIFEST_PATH="${BACKUP_DIR}/manifest.json.prev"
BACKUP_VERSION_PATH="${BACKUP_DIR}/current_version.prev"

# 3. Structured JSON Manifest Parser & Validator (No eval/source, strict destination binding)

find_python_prerequisite() {
  if command -v python3 >/dev/null 2>&1; then
    echo "python3"
  elif command -v python >/dev/null 2>&1; then
    if python -c "import sys; sys.exit(0 if sys.version_info.major >= 3 else 1)" >/dev/null 2>&1; then
      echo "python"
    else
      return 1
    fi
  else
    return 1
  fi
}

# Real structured manifest validator bound to explicit target destination
validate_manifest_with_destination() {
  local manifest_path="$1"
  local exp_prefix="$2"
  local exp_bin_dir="$3"
  local exp_data_dir="$4"
  local exp_bin_path="$5"
  local exp_component="${6:-${COMPONENT_NAME}}"

  local py_bin
  py_bin="$(find_python_prerequisite)" || {
    echo "Error: Python 3 is required for structured JSON manifest validation but was not found in PATH." >&2
    return 1
  }

  if [ ! -f "${manifest_path}" ]; then
    return 1
  fi

  # Symlink escape check on manifest file: must not escape DATA_DIR
  if [ -L "${manifest_path}" ]; then
    local real_file
    real_file="$(readlink -f "${manifest_path}" 2>/dev/null || true)"
    if ! is_path_inside_prefix "${real_file}" "${exp_data_dir}"; then
      echo "Error: Manifest file '${manifest_path}' is a symlink escaping data dir '${exp_data_dir}'." >&2
      return 1
    fi
  fi

  {
    printf '%s\n' "${exp_prefix}"
    printf '%s\n' "${exp_bin_dir}"
    printf '%s\n' "${exp_data_dir}"
    printf '%s\n' "${exp_bin_path}"
    printf '%s\n' "${exp_component}"
    cat "${manifest_path}"
  } | "${py_bin}" -c '
import sys
import json

lines = []
for _ in range(5):
    lines.append(sys.stdin.readline().rstrip("\r\n"))

exp_prefix, exp_bin_dir, exp_data_dir, exp_bin_path, exp_component = lines
raw_manifest = sys.stdin.read()

def detect_duplicate_keys(pairs):
    d = {}
    for k, v in pairs:
        if k in d:
            raise ValueError(f"Duplicate key found in manifest: {k!r}")
        d[k] = v
    return d

try:
    data = json.loads(raw_manifest, object_pairs_hook=detect_duplicate_keys)
except json.JSONDecodeError as e:
    sys.stderr.write(f"Error: Manifest is invalid JSON: {e}\n")
    sys.exit(1)
except ValueError as e:
    sys.stderr.write(f"Error: {e}\n")
    sys.exit(1)

if not isinstance(data, dict):
    sys.stderr.write("Error: Manifest root must be a JSON object\n")
    sys.exit(1)

allowed_keys = {
    "format_version", "component", "version", "installed_at",
    "prefix", "bin_dir", "data_dir", "binary_path", "owned_files", "previous_version"
}
unexpected_keys = set(data.keys()) - allowed_keys
if unexpected_keys:
    sys.stderr.write(f"Error: Unexpected keys in manifest: {unexpected_keys}\n")
    sys.exit(1)

# Format version check: must be integer 1 (not bool)
fmt = data.get("format_version")
if type(fmt) is not int or fmt != 1:
    sys.stderr.write(f"Error: format_version must be integer 1, got {fmt!r}\n")
    sys.exit(1)

# Component check
comp = data.get("component")
if not isinstance(comp, str) or (comp != exp_component and comp != "dosi-tracker-linux"):
    sys.stderr.write(f"Error: component must be {exp_component!r}, got {comp!r}\n")
    sys.exit(1)

# Version check: nonempty string
ver = data.get("version")
if not isinstance(ver, str) or not ver.strip():
    sys.stderr.write("Error: version must be a non-empty string\n")
    sys.exit(1)

# installed_at check: nonempty string
inst = data.get("installed_at")
if not isinstance(inst, str) or not inst.strip():
    sys.stderr.write("Error: installed_at must be a non-empty string\n")
    sys.exit(1)

# prefix check: exact canonical match
pref = data.get("prefix")
if not isinstance(pref, str) or pref != exp_prefix:
    sys.stderr.write(f"Error: prefix mismatch: expected {exp_prefix!r}, got {pref!r}\n")
    sys.exit(1)

# bin_dir check: exact canonical match
b_dir = data.get("bin_dir")
if not isinstance(b_dir, str) or b_dir != exp_bin_dir:
    sys.stderr.write(f"Error: bin_dir mismatch: expected {exp_bin_dir!r}, got {b_dir!r}\n")
    sys.exit(1)

# data_dir check: exact canonical match
d_dir = data.get("data_dir")
if not isinstance(d_dir, str) or d_dir != exp_data_dir:
    sys.stderr.write(f"Error: data_dir mismatch: expected {exp_data_dir!r}, got {d_dir!r}\n")
    sys.exit(1)

# binary_path check: exact match or with .exe on Windows
bin_p = data.get("binary_path")
if not isinstance(bin_p, str):
    sys.stderr.write("Error: binary_path must be a string\n")
    sys.exit(1)

if bin_p != exp_bin_path and bin_p != (exp_bin_path + ".exe") and (bin_p + ".exe") != exp_bin_path:
    sys.stderr.write(f"Error: binary_path mismatch: expected {exp_bin_path!r}, got {bin_p!r}\n")
    sys.exit(1)

# owned_files check: non-empty array of strings
owned = data.get("owned_files")
if not isinstance(owned, list) or len(owned) == 0:
    sys.stderr.write("Error: owned_files must be a non-empty array\n")
    sys.exit(1)

# Platform binary identity: candidate can be exp_bin_path or exp_bin_path.exe
valid_bin_candidates = {exp_bin_path}
if exp_bin_path.endswith(".exe"):
    valid_bin_candidates.add(exp_bin_path[:-4])
else:
    valid_bin_candidates.add(exp_bin_path + ".exe")

# Exact bounded allowed inventory for installer-owned files (unambiguous binary, live manifest, live version)
allowed_owned = set(valid_bin_candidates)
allowed_owned.add(f"{exp_data_dir}/manifest.json")
allowed_owned.add(f"{exp_data_dir}/current_version")

prefix_boundary = exp_prefix.rstrip("/") + "/"
seen_owned = set()

import posixpath
import os

for item in owned:
    if not isinstance(item, str):
        sys.stderr.write(f"Error: Entry in owned_files is not a string: {item!r}\n")
        sys.exit(1)

    # Reject control characters (e.g. \n, \r, \t, \0, DEL)
    if any(ord(c) < 32 or ord(c) == 127 for c in item):
        sys.stderr.write(f"Error: Control character detected in owned_files entry: {item!r}\n")
        sys.exit(1)

    # Reject leading/trailing whitespace tricks
    if item != item.strip():
        sys.stderr.write(f"Error: Leading or trailing whitespace in owned_files entry: {item!r}\n")
        sys.exit(1)

    # Reject non-canonical segments (//, /./, /../, trailing slash, relative paths)
    if not item.startswith("/"):
        sys.stderr.write(f"Error: owned_files entry must be an absolute path: {item!r}\n")
        sys.exit(1)

    if item.endswith("/") or "//" in item:
        sys.stderr.write(f"Error: Non-canonical path in owned_files entry: {item!r}\n")
        sys.exit(1)

    parts = item.split("/")
    if any(p in (".", "..") for p in parts):
        sys.stderr.write(f"Error: Traversal or dot segment in owned_files entry: {item!r}\n")
        sys.exit(1)

    # Strict prefix confinement
    if not item.startswith(prefix_boundary):
        sys.stderr.write(f"Error: owned file {item!r} is outside prefix {exp_prefix!r}\n")
        sys.exit(1)

    # Strict allowed inventory check: only actual binary, manifest, and current_version are allowed
    if item not in allowed_owned:
        sys.stderr.write(f"Error: Entry in owned_files is not an allowed installer-managed file: {item!r}\n")
        sys.exit(1)

    # Symlink escape check on parent directory components if they exist on disk
    curr = posixpath.dirname(item)
    while curr and curr != exp_prefix and curr.startswith(prefix_boundary):
        if os.path.islink(curr):
            try:
                resolved = os.path.realpath(curr)
                if not (resolved == exp_prefix or resolved.startswith(prefix_boundary)):
                    sys.stderr.write(f"Error: Parent directory of owned file is a symlink escaping prefix: {curr!r} -> {resolved!r}\n")
                    sys.exit(1)
            except Exception:
                pass
        curr = posixpath.dirname(curr)

    if item in seen_owned:
        sys.stderr.write(f"Error: Duplicate file in owned_files array: {item!r}\n")
        sys.exit(1)
    seen_owned.add(item)

# binary_path MUST be explicitly in owned_files array
if bin_p not in seen_owned:
    sys.stderr.write(f"Error: binary_path {bin_p!r} is not contained in owned_files array\n")
    sys.exit(1)

# previous_version: null or nonempty string
prev_ver = data.get("previous_version")
if prev_ver is not None and (not isinstance(prev_ver, str) or not prev_ver.strip()):
    sys.stderr.write(f"Error: previous_version must be null or non-empty string, got {prev_ver!r}\n")
    sys.exit(1)

sys.exit(0)
'
}

# Backward compatible helper: delegates to destination-bound validator
validate_manifest_structure() {
  local file="$1"
  local dest_bin="${2:-${TARGET_BIN_PATH}}"
  validate_manifest_with_destination "${file}" "${CANONICAL_PREFIX}" "${BIN_DIR}" "${DATA_DIR}" "${dest_bin}"
}

# Verify manifest ownership of an existing binary
verify_manifest_ownership() {
  local existing_bin="$1"
  if [ ! -f "${MANIFEST_PATH}" ]; then
    return 1
  fi

  local target_bin="${existing_bin}"
  if [ -z "${target_bin}" ]; then
    target_bin="${TARGET_BIN_PATH}"
  fi

  if ! validate_manifest_with_destination "${MANIFEST_PATH}" "${CANONICAL_PREFIX}" "${BIN_DIR}" "${DATA_DIR}" "${target_bin}"; then
    return 1
  fi

  # Symlink escape check on existing binary
  if [ -n "${existing_bin}" ] && [ -L "${existing_bin}" ]; then
    local b_real
    b_real="$(readlink -f "${existing_bin}" 2>/dev/null || true)"
    if ! is_path_inside_prefix "${b_real}" "${BIN_DIR}"; then
      return 1
    fi
  fi

  return 0
}

# Helper: parse single field from manifest safely via JSON
get_manifest_field() {
  local manifest_file="$1"
  local field_name="$2"
  local py_bin
  py_bin="$(find_python_prerequisite)" || return 1

  {
    printf '%s\n' "${field_name}"
    cat "${manifest_file}"
  } | "${py_bin}" -c '
import sys
import json

field = sys.stdin.readline().rstrip("\r\n")
try:
    data = json.load(sys.stdin)
    val = data.get(field)
    if val is not None:
        if isinstance(val, (dict, list)):
            print(json.dumps(val))
        else:
            print(val)
except Exception:
    sys.exit(1)
'
}

# Helper: extract list of owned files from manifest safely via JSON as null-delimited stream
get_manifest_owned_files() {
  local manifest_file="$1"
  local py_bin
  py_bin="$(find_python_prerequisite)" || return 1

  cat "${manifest_file}" | "${py_bin}" -c '
import sys
import json

try:
    data = json.load(sys.stdin)
    for f in data.get("owned_files", []):
        if isinstance(f, str) and f:
            # Emit raw UTF-8 bytes followed by NUL byte (no stripping, no newline splitting)
            sys.stdout.buffer.write(f.encode("utf-8") + b"\0")
except Exception:
    sys.exit(1)
'
}

# Helper: generate manifest JSON safely with proper escaping and input validation
generate_manifest_json() {
  local out_file="$1"
  local component="$2"
  local version="$3"
  local installed_at="$4"
  local prefix="$5"
  local bin_dir="$6"
  local data_dir="$7"
  local binary_path="$8"
  local manifest_path="$9"
  local version_path="${10}"
  local prev_version="${11}"

  local py_bin
  py_bin="$(find_python_prerequisite)" || {
    echo "Error: Python 3 is required for JSON manifest generation." >&2
    return 1
  }

  {
    printf '%s\n' "${component}"
    printf '%s\n' "${version}"
    printf '%s\n' "${installed_at}"
    printf '%s\n' "${prefix}"
    printf '%s\n' "${bin_dir}"
    printf '%s\n' "${data_dir}"
    printf '%s\n' "${binary_path}"
    printf '%s\n' "${manifest_path}"
    printf '%s\n' "${version_path}"
    printf '%s\n' "${prev_version}"
  } | "${py_bin}" -c '
import sys
import json

lines = []
for _ in range(10):
    lines.append(sys.stdin.readline().rstrip("\r\n"))

component, version, installed_at, prefix, bin_dir, data_dir, binary_path, manifest_path, version_path, prev_version = lines

# Reject unsupported characters (e.g. control characters, null bytes) before mutation
for val, name in [
    (component, "component"),
    (version, "version"),
    (prefix, "prefix"),
    (bin_dir, "bin_dir"),
    (data_dir, "data_dir"),
    (binary_path, "binary_path"),
    (manifest_path, "manifest_path"),
    (version_path, "version_path"),
]:
    if any(ord(c) < 32 or ord(c) == 127 for c in val):
        sys.stderr.write(f"Error: Invalid control character or null byte in {name}\n")
        sys.exit(1)

prev_v = prev_version if prev_version != "none" and prev_version != "null" and prev_version != "" else None

data = {
    "format_version": 1,
    "component": component,
    "version": version,
    "installed_at": installed_at,
    "prefix": prefix,
    "bin_dir": bin_dir,
    "data_dir": data_dir,
    "binary_path": binary_path,
    "owned_files": [
        binary_path,
        manifest_path,
        version_path
    ],
    "previous_version": prev_v
}

sys.stdout.write(json.dumps(data, indent=2) + "\n")
' > "${out_file}"
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

  echo "==> Building dosi-tracker (${CRATE_VERSION}) via cargo --release..." >&2
  cargo build --locked --release --manifest-path "${LINUX_CRATE_DIR}/Cargo.toml" --bin dosi-tracker >&2

  # Cargo honours CARGO_TARGET_DIR (CI keeps its build in a separate volume).
  local target_dir="${CARGO_TARGET_DIR:-${LINUX_CRATE_DIR}/target}"
  local cand1="${target_dir}/release/dosi-tracker"
  local cand2="${target_dir}/release/dosi-tracker.exe"
  if [ -f "${cand1}" ]; then
    echo "${cand1}"
  elif [ -f "${cand2}" ]; then
    echo "${cand2}"
  else
    echo "Error: Built binary not found in ${target_dir}/release" >&2
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

  # Remove owned files explicitly recorded in manifest
  if [ -f "${MANIFEST_PATH}" ]; then
    local file_to_remove
    while IFS= read -r -d '' file_to_remove; do
      if [ -n "${file_to_remove}" ]; then
        # Independent traversal / non-canonical guard in bash
        if [[ "${file_to_remove}" == *$'\n'* ]] || [[ "${file_to_remove}" == *$'\r'* ]] || [[ "${file_to_remove}" == *"/../"* ]] || [[ "${file_to_remove}" == *".."* ]]; then
          echo "Error: Refusing to delete non-canonical or traversal path '${file_to_remove}'." >&2
          exit 1
        fi

        # Independent confinement guard in bash
        if ! is_path_inside_prefix "${file_to_remove}" "${CANONICAL_PREFIX}"; then
          echo "Error: Manifest entry '${file_to_remove}' escapes canonical prefix '${CANONICAL_PREFIX}'." >&2
          exit 1
        fi

        # Independent inventory allowlist guard in bash
        case "${file_to_remove}" in
          "${TARGET_BIN_PATH}"|"${TARGET_BIN_EXE_PATH}"|"${MANIFEST_PATH}"|"${CURRENT_VERSION_PATH}")
            if [ -f "${file_to_remove}" ] || [ -L "${file_to_remove}" ]; then
              rm -f "${file_to_remove}"
              echo "Removed owned file: ${file_to_remove}"
            fi
            ;;
          *)
            echo "Error: Refusing to delete unexpected file '${file_to_remove}' not in installer inventory." >&2
            exit 1
            ;;
        esac
      fi
    done < <(get_manifest_owned_files "${MANIFEST_PATH}")
  elif [ -n "${cur_bin}" ] && [ -f "${cur_bin}" ]; then
    rm -f "${cur_bin}"
    echo "Removed binary: ${cur_bin}"
  fi

  # Remove backups safely (guard against symlink escapes)
  if [ -d "${BACKUP_DIR}" ] && [ ! -L "${BACKUP_DIR}" ]; then
    rm -f "${BACKUP_BIN_PATH}" "${BACKUP_BIN_EXE_PATH}" "${BACKUP_MANIFEST_PATH}" "${BACKUP_VERSION_PATH}" 2>/dev/null || true
    rmdir "${BACKUP_DIR}" 2>/dev/null || true
  elif [ -L "${BACKUP_DIR}" ]; then
    rm -f "${BACKUP_DIR}" 2>/dev/null || true
  fi

  # Remove current_version
  rm -f "${CURRENT_VERSION_PATH}" 2>/dev/null || true

  # Remove manifest
  rm -f "${MANIFEST_PATH}" 2>/dev/null || true
  echo "Removed manifest: ${MANIFEST_PATH}"

  # Clean data dir if empty
  if [ -d "${DATA_DIR}" ] && [ ! -L "${DATA_DIR}" ]; then
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

  local dest_bin="${TARGET_BIN_PATH}"
  if [[ "${backup_bin}" == *.exe.prev ]]; then
    dest_bin="${TARGET_BIN_EXE_PATH}"
  fi

  # Check symlink escapes on backup files
  if [ -L "${backup_bin}" ] || [ -L "${BACKUP_MANIFEST_PATH}" ]; then
    echo "Error: Backup file is a symlink. Rollback aborted." >&2
    exit 1
  fi

  # Validate backup manifest with destination binding
  if ! validate_manifest_with_destination "${BACKUP_MANIFEST_PATH}" "${CANONICAL_PREFIX}" "${BIN_DIR}" "${DATA_DIR}" "${dest_bin}"; then
    echo "Error: Backup manifest is corrupted, invalid, or does not match target prefix/binary. Rollback aborted." >&2
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

  generate_manifest_json \
    "${staged_manifest}" \
    "${COMPONENT_NAME}" \
    "${detected_version}" \
    "${timestamp}" \
    "${CANONICAL_PREFIX}" \
    "${BIN_DIR}" \
    "${DATA_DIR}" \
    "${final_bin}" \
    "${MANIFEST_PATH}" \
    "${CURRENT_VERSION_PATH}" \
    "${prev_version}"

  echo "${detected_version}" > "${staged_version}"

  # Validate the staged manifest structure and destination binding before touching live files
  if ! validate_manifest_with_destination "${staged_manifest}" "${CANONICAL_PREFIX}" "${BIN_DIR}" "${DATA_DIR}" "${final_bin}"; then
    echo "Error: Staged manifest failed structured validation." >&2
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
