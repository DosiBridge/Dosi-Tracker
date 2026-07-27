#!/usr/bin/env bash
#
# Merges the per-test-project Cobertura reports produced by `dotnet test --collect` into a single
# report and fails if line/branch coverage is below the ratcheting floor from docs/QUALITY.md.
#
# Usage:  backend/scripts/coverage-gate.sh [LINE_MIN] [BRANCH_MIN]
# Run from the repo root. Requires: dotnet-reportgenerator-globaltool, python3.
#
# The floors below are the Phase 1 gate. To raise them: land tests that clear the new bar, then
# bump these defaults + the QUALITY.md table in the same PR. Never lower them.
set -euo pipefail

LINE_MIN="${1:-80}"
BRANCH_MIN="${2:-45}"
RESULTS_DIR="${COVERAGE_RESULTS_DIR:-backend/TestResults}"
MERGED_DIR="${RESULTS_DIR}/merged"

shopt -s globstar nullglob
reports=("${RESULTS_DIR}"/**/coverage.cobertura.xml)
if [ ${#reports[@]} -eq 0 ]; then
  echo "::error::No coverage.cobertura.xml found under ${RESULTS_DIR}. Did 'dotnet test --collect' run?"
  exit 1
fi
echo "Found ${#reports[@]} coverage report(s); merging…"

# ReportGenerator dedupes overlapping assemblies correctly (max coverage per line across reports),
# so the merged number is honest even though Domain is measured by several test assemblies.
reportgenerator \
  "-reports:${RESULTS_DIR}/**/coverage.cobertura.xml" \
  "-targetdir:${MERGED_DIR}" \
  "-reporttypes:Cobertura;TextSummary" >/dev/null

python3 - "$MERGED_DIR/Cobertura.xml" "$LINE_MIN" "$BRANCH_MIN" <<'PY'
import os, sys, xml.etree.ElementTree as ET
path, line_min, branch_min = sys.argv[1], float(sys.argv[2]), float(sys.argv[3])
root = ET.parse(path).getroot()
line = float(root.get('line-rate') or 0) * 100
branch = float(root.get('branch-rate') or 0) * 100
print(f"::notice::Merged coverage - line={line:.2f}% (floor {line_min}%)  branch={branch:.2f}% (floor {branch_min}%)")
ok = line >= line_min and branch >= branch_min
if line < line_min:
    print(f"::error::Line coverage {line:.2f}% is below the floor of {line_min}%")
if branch < branch_min:
    print(f"::error::Branch coverage {branch:.2f}% is below the floor of {branch_min}%")
if ok:
    print(f"Coverage gate PASSED: line {line:.2f}% >= {line_min}%, branch {branch:.2f}% >= {branch_min}%")

# Quality metrics dashboard: surface the numbers in the GitHub Actions run summary.
summary = os.environ.get('GITHUB_STEP_SUMMARY')
if summary:
    verdict = '✅ PASSED' if ok else '❌ FAILED'
    with open(summary, 'a', encoding='utf-8') as fh:
        fh.write(f"## Backend coverage gate — {verdict}\n\n")
        fh.write("| Metric | Coverage | Floor |\n|---|---:|---:|\n")
        fh.write(f"| Line | {line:.2f}% | {line_min:.0f}% |\n")
        fh.write(f"| Branch | {branch:.2f}% | {branch_min:.0f}% |\n\n")
        fh.write("| Assembly | Line | Branch |\n|---|---:|---:|\n")
        for pkg in sorted(root.iter('package'), key=lambda p: p.get('name') or ''):
            n = pkg.get('name') or ''
            pl = float(pkg.get('line-rate') or 0) * 100
            pb = float(pkg.get('branch-rate') or 0) * 100
            fh.write(f"| {n} | {pl:.1f}% | {pb:.1f}% |\n")
sys.exit(0 if ok else 1)
PY
