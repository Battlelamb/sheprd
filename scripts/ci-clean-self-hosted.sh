#!/usr/bin/env bash
# A self-hosted runner's checkout, cleaned like a fresh one but for `target/`,
# so the next build reuses what the last one compiled. The directory is
# dropped once it outgrows SHEPRD_CI_TARGET_LIMIT_GB (default 80), as feature
# sets, profiles, and toolchains pile up in it.
set -euo pipefail

git clean -ffdx -e /target/
git reset --hard --quiet HEAD

limit_gb="${SHEPRD_CI_TARGET_LIMIT_GB:-80}"
[[ "$limit_gb" =~ ^[0-9]+$ ]] || { echo "SHEPRD_CI_TARGET_LIMIT_GB must be a whole number" >&2; exit 1; }
if [[ -d target ]]; then
  used_kb=$(du -sk target | cut -f1)
  if (( used_kb > limit_gb * 1024 * 1024 )); then
    echo "target/ holds $((used_kb / 1024 / 1024)) GB, over ${limit_gb} GB: removing it"
    rm -rf target
  fi
fi
