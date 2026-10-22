#!/bin/zsh

set -euo pipefail

script_dir=${0:A:h}
project_dir=${script_dir:h}

fail() {
  print -u2 -- "ERROR: $1"
  exit 1
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || fail "Required command is unavailable: $1"
}

run_step() {
  local label=$1
  local exit_code
  shift

  print -- "$label"
  if "$@"; then
    return 0
  else
    exit_code=$?
  fi

  print -u2 -- "FAILED: $label (exit $exit_code)"
  return "$exit_code"
}

require_command npm
require_command node

[[ -f "$project_dir/frontend/package.json" ]] || \
  fail "Frontend package manifest is missing: frontend/package.json"
[[ -f "$project_dir/frontend/package-lock.json" ]] || \
  fail "Frontend lockfile is missing: frontend/package-lock.json"

run_step "Running backend verification..." "$script_dir/verify-backend.sh"

cd "$project_dir/frontend"
run_step "Running frontend tests..." npm test
run_step "Building the frontend..." npm run build

echo "All verification checks passed."

