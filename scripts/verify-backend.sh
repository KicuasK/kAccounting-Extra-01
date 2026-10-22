#!/bin/zsh

set -euo pipefail

script_dir=${0:A:h}
project_dir=${script_dir:h}

cd "$project_dir"

echo "Checking source/runtime data separation..."
runtime_data_files=$(find "$project_dir" \
  \( -path "$project_dir/.git" -o -path "$project_dir/target" -o -path "$project_dir/frontend/node_modules" \) -prune \
  -o \( -type f -o -type l -o -type d \) \
    \( -name '*.sqlite*' -o -name '*.db' -o -name '*.db-*' -o -name '*.db3' -o -name '*.db3-*' \
       -o -name '*.kagextra' -o -name '*.kagextra.partial' \
       -o -name '.kAccounting-Extra-01-backup-*' -o -name '.kAccounting-Extra-01-object-*' \) \
    -print)
if [[ -n "$runtime_data_files" ]]; then
  echo "Runtime databases, transfer packages, and private work state must not be stored in the source repository:" >&2
  echo "$runtime_data_files" >&2
  exit 1
fi

echo "Checking accounting money types..."
money_type_violations=$(rg -n '\b(f32|f64)\b|\bREAL\b' \
  "$project_dir/backend/src/accounting" \
  "$project_dir/backend/migrations" || true)
if [[ -n "$money_type_violations" ]]; then
  echo "Accounting code and migrations must not use binary floating-point money types:" >&2
  echo "$money_type_violations" >&2
  exit 1
fi

echo "Checking Rust safety profile..."
if ! rg -q '^overflow-checks = true$' "$project_dir/Cargo.toml"; then
  echo "Release builds must keep integer overflow checks enabled." >&2
  exit 1
fi
for crate_root in \
  "$project_dir/backend/src/lib.rs" \
  "$project_dir/backend/src/main.rs"; do
  if ! rg -q '^#!\[forbid\(unsafe_code\)\]$' "$crate_root"; then
    echo "Rust crate roots must forbid unsafe code: $crate_root" >&2
    exit 1
  fi
done

echo "Checking private-service boundary..."
public_service_violations=$(rg -n -i \
  '0\.0\.0\.0|\[::\]|Ipv4Addr::UNSPECIFIED|Ipv6Addr::UNSPECIFIED|tailscale[[:space:]_-]+funnel|funnel[[:space:]_-]+on' \
  "$project_dir/backend" || true)
if [[ -n "$public_service_violations" ]]; then
  echo "Public listener or Tailscale Funnel configuration is prohibited:" >&2
  echo "$public_service_violations" >&2
  exit 1
fi

if ! rg -q '^axum = \{[^}]*default-features = false' "$project_dir/backend/Cargo.toml"; then
  echo "Axum server features must remain opt-in until serving policy is approved." >&2
  exit 1
fi

network_startup_violations=$(rg -n \
  'TcpListener|axum::serve|hyper::server|tokio::net|std::net' \
  "$project_dir/backend/src" || true)
if [[ -n "$network_startup_violations" ]]; then
  echo "Network startup code requires an approved bind and authentication policy:" >&2
  echo "$network_startup_violations" >&2
  exit 1
fi

server_dependency_violations=$(rg -n \
  '^(hyper|hyper-util|axum-server)[[:space:]]*=|^axum = .*features = \[[^]]*"(http1|http2)"' \
  "$project_dir/backend/Cargo.toml" || true)
if [[ -n "$server_dependency_violations" ]]; then
  echo "HTTP serving dependencies and features are not approved yet:" >&2
  echo "$server_dependency_violations" >&2
  exit 1
fi

enabled_server_features=$(cargo tree --locked -e features -i axum \
  | rg 'axum feature "(http1|http2)"' || true)
if [[ -n "$enabled_server_features" ]]; then
  echo "Axum HTTP serving features are not approved yet:" >&2
  echo "$enabled_server_features" >&2
  exit 1
fi

echo "Checking Rust formatting..."
cargo fmt --all -- --check

echo "Running Rust lints..."
cargo clippy --workspace --all-targets --locked -- -D warnings

echo "Checking production Rust for panic shortcuts..."
cargo clippy --workspace --lib --bins --locked -- \
  -D warnings \
  -W clippy::unwrap_used \
  -W clippy::expect_used \
  -W clippy::panic

echo "Running Rust tests..."
cargo test --workspace --all-targets --locked
cargo test --doc --locked

echo "Checking Rust documentation..."
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --document-private-items --locked

echo "All backend verification checks passed."
