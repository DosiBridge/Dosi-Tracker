#!/usr/bin/env bash
#
# Rootless uninstaller wrapper for Dosi-Tracker Linux client.
# Delegates to install.sh uninstall.
#

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec "${SCRIPT_DIR}/install.sh" uninstall "$@"
