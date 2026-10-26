#!/usr/bin/env bash
# Decides whether VERSION is the highest known release, so release.yml's `images` job knows whether the GHCR images
# it pushes should also get the floating `:latest` tag.
#
# Why this exists: release.yml's `version` job documents workflow_dispatch's `version` input as a supported path for
# re-running the pipeline against an *older* release (e.g. re-signing a Windows installer, or a GHCR-only re-push).
# Docker registry tags have no version ordering -- whichever push happens last wins `:latest` unconditionally -- so
# without this check, re-running the workflow for an old version would silently regress every self-hoster who tracks
# `:latest` (docs/SELF_HOSTING.md section 8) to an older, possibly-vulnerable image.
#
# Compares VERSION against every `vX.Y.Z` tag already reachable in the checkout, plus VERSION itself, using semver
# sort (`sort -V`). VERSION is latest when nothing beats it; a tie (re-running the current highest version) counts
# as latest too. Requires the full tag history (`actions/checkout` with `fetch-depth: 0` and `fetch-tags: true`);
# with no tag history at all, VERSION is trivially the only candidate and is "true".
#
# Usage: VERSION=1.2.3 bash .github/scripts/is-latest-release.sh
# Prints "true" or "false" to stdout.
set -euo pipefail

: "${VERSION:?set VERSION (e.g. 1.2.3, no leading v)}"

highest="$(
  { git tag --list 'v*' 2>/dev/null | sed -E 's/^v//'; printf '%s\n' "${VERSION}"; } |
    grep -E '^[0-9]+\.[0-9]+\.[0-9]+$' |
    sort -V | tail -1
)"

if [[ "${highest}" == "${VERSION}" ]]; then
  echo "true"
else
  echo "false"
fi
