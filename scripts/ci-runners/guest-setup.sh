#!/bin/bash
# Runs inside a CI VM as its runner user, copied there by host.sh: what
# ci.yml's jobs need that the Cirrus Labs base images lack, and the PATH the
# runner service starts jobs with. Safe to run again.
set -euo pipefail

if [[ "$(uname -s)" == Darwin ]]; then
  export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/opt/homebrew/sbin:$PATH"
  export HOMEBREW_NO_AUTO_UPDATE=1 HOMEBREW_NO_INSTALL_CLEANUP=1
  # The update manifest tests read `brew --prefix openssl@3`; the release
  # script tests need Python 3.11 or later, ahead of the system's.
  brew list openssl@3 >/dev/null 2>&1 || brew install --quiet openssl@3 </dev/null
  python3 -c 'import sys; sys.exit(sys.version_info < (3, 11))' 2>/dev/null || brew install --quiet python </dev/null
  command -v jq >/dev/null || brew install --quiet jq </dev/null
  job_path="$HOME/.cargo/bin:/opt/homebrew/bin:/opt/homebrew/sbin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"
  limit=100
  service() { ./svc.sh "$1"; }
else
  # scripts/install-linux-deps.sh runs in every job and needs it.
  sudo -n true || { echo "passwordless sudo is required" >&2; exit 1; }
  export DEBIAN_FRONTEND=noninteractive
  # Grow the root file system into a disk `tart set` enlarged.
  sudo growpart /dev/vda 1 >/dev/null 2>&1 || true
  sudo resize2fs /dev/vda1 >/dev/null 2>&1 || true
  sudo apt-get update -qq
  sudo apt-get install -y -qq --no-install-recommends build-essential git curl ca-certificates python3 jq >/dev/null
  job_path="$HOME/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
  limit=70
  service() { sudo ./svc.sh "$1"; }
fi

if [[ ! -x "$HOME/.cargo/bin/rustup" ]]; then
  # The toolchain itself comes from rust-toolchain.toml on the first build.
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs |
    sh -s -- -y --no-modify-path --default-toolchain none --profile minimal >/dev/null 2>&1
fi

cd "$HOME/actions-runner"
printf '%s\n' "$job_path" >.path
grep -q '^SHEPRD_CI_TARGET_LIMIT_GB=' .env 2>/dev/null || echo "SHEPRD_CI_TARGET_LIMIT_GB=$limit" >>.env
# The runner reads .path and .env when it starts.
service stop >/dev/null 2>&1 || true
service start >/dev/null
echo "$("$HOME/.cargo/bin/rustup" --version 2>/dev/null | head -1), $(PATH="$job_path" python3 --version), $(df -h / | tail -1 | awk '{print $4}') free"
