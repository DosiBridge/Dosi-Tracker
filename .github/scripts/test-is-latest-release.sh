#!/usr/bin/env bash
# Unit tests for is-latest-release.sh, run on every PR (ci.yml's release-scripts job) so a regression in the
# release workflow's `:latest` gating is caught without needing a real release.
#
# Each case builds a throwaway git repo with a synthetic tag history and checks the script's verdict for a given
# VERSION. The case that matters most is "re-run an older version by workflow_dispatch after a newer one shipped"
# (case 3): that is exactly the scenario the script exists to stop from tagging GHCR images `:latest`.
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
script="${script_dir}/is-latest-release.sh"
work_dir="$(mktemp -d)"
trap 'rm -rf "${work_dir}"' EXIT

failures=0

# Builds a fresh git repo at $work_dir/repo with one commit and the given tags (any number, in order).
make_repo() {
  rm -rf "${work_dir}/repo"
  git init -q "${work_dir}/repo"
  (
    cd "${work_dir}/repo"
    git -c user.name=test -c user.email=test@test.invalid commit -q --allow-empty -m init
    for tag in "$@"; do
      git tag "${tag}"
    done
  )
}

check() {
  local description="$1" version="$2" expected="$3" actual
  actual="$(cd "${work_dir}/repo" && VERSION="${version}" bash "${script}")"
  if [[ "${actual}" == "${expected}" ]]; then
    echo "ok - ${description} (VERSION=${version} -> ${actual})"
  else
    echo "FAIL - ${description}: VERSION=${version} expected ${expected}, got ${actual}" >&2
    failures=$((failures + 1))
  fi
}

# 1. No release has ever shipped: the first version is trivially the latest.
make_repo
check "first release ever, no tags" "1.0.0" "true"

# 2. A brand new highest version, released by workflow_dispatch before the tag exists (the normal push: tags path
#    creates the tag first, but workflow_dispatch's documented re-run path does not).
make_repo v1.0.0 v1.1.0
check "new highest version, tag not created yet" "1.2.0" "true"

# 3. THE BUG THIS SCRIPT FIXES: re-running the workflow for an older version after a newer one has already shipped
#    (e.g. to re-sign the Windows installer) must not win `:latest`.
make_repo v1.0.0 v1.1.0 v1.2.0
check "re-run an older version after a newer one shipped" "1.1.0" "false"

# 4. Re-running the *current* highest version (e.g. a GHCR-only re-push) is still latest.
make_repo v1.0.0 v1.1.0 v1.2.0
check "re-run the current highest version" "1.2.0" "true"

# 5. Double-digit components must sort numerically, not lexically (sort -V, not sort).
make_repo v1.9.0 v1.10.0
check "numeric version sort (1.10.0 beats 1.9.0)" "1.9.0" "false"
check "numeric version sort (1.10.0 is latest)" "1.10.0" "true"

if [[ "${failures}" -gt 0 ]]; then
  echo "${failures} test(s) failed" >&2
  exit 1
fi
echo "all is-latest-release.sh tests passed"
