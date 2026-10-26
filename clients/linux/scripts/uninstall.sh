#!/usr/bin/env bash
#
# Rootless uninstaller wrapper for the Dosi Tracker Linux agent (tarball and source
# installs). Delegates to install.sh uninstall; options: --prefix DIR, --purge, --yes.
#
# An installation also carries its own copy at <prefix>/lib/dosi-tracker/install/uninstall.sh
# (which is what `dosi-tracker uninstall` runs).
#

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec bash "${SCRIPT_DIR}/install.sh" uninstall "$@"
