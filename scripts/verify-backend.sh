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

report_violations() {
  local message=$1
  local violations=$2

  print -u2 -- "ERROR: $message"
  print -u2 -- "$violations"
  exit 1
}

reject_rg_matches() {
  local violation_message=$1
  local scan_description=$2
  local matches
  local exit_code
  shift 2

  if matches=$(rg "$@"); then
    report_violations "$violation_message" "$matches"
  else
    exit_code=$?
  fi

  (( exit_code == 1 )) || \
    fail "Could not scan $scan_description (rg exit $exit_code)."
}

require_command cargo
require_command find
require_command rg

cd "$project_dir"

echo "Checking source/runtime data separation..."
if ! runtime_data_files=$(find "$project_dir" \
  \( -path "$project_dir/.git" -o -path "$project_dir/target" -o -path "$project_dir/frontend/node_modules" -o -path "$project_dir/frontend/dist" \) -prune \
  -o \( -type f -o -type l -o -type d \) \
    \( -name '*.sqlite*' -o -name '*.db' -o -name '*.db-*' -o -name '*.db3' -o -name '*.db3-*' \
       -o -name '*.kabackup' -o -name '*.kae' -o -name '*.kagextra' -o -name '*.kagextra.partial' \
       -o -name '.kAccounting-Extra-01-backup-*' -o -name '.kAccounting-Extra-01-object-*' \
       -o -name '.kAccounting-Extra-01-encrypted-backup-*.work' -o -name '.kAccounting-Extra-01-export-*.work' -o -name '.kAccounting-Extra-01-snapshot-*.work' \
       -o -name '.kAccounting-Extra-01-init-*' -o -name '*.managed' \
       -o -name '.runtime.lock' -o -name '.maintenance.lock' -o -name '.backup-service.lock' -o -name 'device-key.json' \
       -o -name 'operation-state.json' -o -name '.operation-state.lock' -o -name '.operation-state-*.tmp' \
       -o -path "$project_dir/backups" \) \
    -print); then
  fail "Could not inspect the source tree for runtime financial data."
fi
if [[ -n "$runtime_data_files" ]]; then
  report_violations \
    "Runtime databases, backups, transfer packages, and private work state must remain outside the source repository. Move these paths out of the checkout:" \
    "$runtime_data_files"
fi

echo "Checking accounting money types..."
reject_rg_matches \
  "Accounting code and migrations must not use binary floating-point money types:" \
  "accounting money types" \
  -n '\b(f32|f64)\b|\bREAL\b' \
  "$project_dir/backend/src/accounting" \
  "$project_dir/backend/migrations"

echo "Checking Rust safety profile..."
if ! rg -q '^overflow-checks = true$' "$project_dir/Cargo.toml"; then
  fail "Release builds must keep integer overflow checks enabled."
fi
for crate_root in \
  "$project_dir/backend/src/lib.rs" \
  "$project_dir/backend/src/main.rs"; do
  if ! rg -q '^#!\[forbid\(unsafe_code\)\]$' "$crate_root"; then
    fail "Rust crate roots must forbid unsafe code: $crate_root"
  fi
done

echo "Checking private-service boundary..."
reject_rg_matches \
  "Public listener or Tailscale Funnel configuration is prohibited:" \
  "the private-service boundary" \
  -n -i \
  '0\.0\.0\.0|\[::\]|Ipv4Addr::UNSPECIFIED|Ipv6Addr::UNSPECIFIED|tailscale[[:space:]_-]+funnel|funnel[[:space:]_-]+on' \
  "$project_dir/backend"

if ! rg -q '^axum = \{[^}]*default-features = false' "$project_dir/backend/Cargo.toml"; then
  fail "Axum default features must remain explicitly disabled."
fi

reject_rg_matches \
  "Network startup must stay inside the tested private runtime boundary:" \
  "network startup outside runtime.rs" \
  -n \
  --glob '!runtime.rs' \
  'TcpListener|axum::serve|hyper::server|tokio::net' \
  "$project_dir/backend/src"

[[ -f "$project_dir/backend/tests/runtime.rs" ]] || \
  fail "Private runtime boundary tests are required before enabling serving."
rg -q '127\.0\.0\.1:3000' "$project_dir/backend/src/runtime.rs" || \
  fail "Runtime must bind to the approved fixed loopback address."

if ! axum_feature_tree=$(cargo tree --locked -e features -i axum); then
  fail "Could not inspect enabled Axum features with cargo tree."
fi
if enabled_server_features=$(rg 'axum feature "http2"' \
  <<< "$axum_feature_tree"); then
  report_violations \
    "Only HTTP/1 serving is approved for the private runtime:" \
    "$enabled_server_features"
else
  feature_scan_exit_code=$?
  (( feature_scan_exit_code == 1 )) || \
    fail "Could not scan enabled Axum features (rg exit $feature_scan_exit_code)."
fi

run_step "Checking Rust formatting..." cargo fmt --all -- --check
run_step "Running Rust lints..." cargo clippy --workspace --all-targets --locked -- -D warnings
run_step "Checking production Rust for panic shortcuts..." \
  cargo clippy --workspace --lib --bins --locked -- \
  -D warnings \
  -W clippy::unwrap_used \
  -W clippy::expect_used \
  -W clippy::panic

run_step "Running Rust tests..." cargo test --workspace --all-targets --locked
run_step "Running Rust documentation tests..." cargo test --doc --locked
run_step "Checking Rust documentation..." \
  env RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --document-private-items --locked

echo "All backend verification checks passed."
