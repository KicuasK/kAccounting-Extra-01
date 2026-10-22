#!/bin/zsh

set -euo pipefail

script_dir=${0:A:h}
project_dir=${script_dir:h}

cd "$project_dir"

echo "Checking source/runtime data separation..."
runtime_data_files=$(find "$project_dir" \
  \( -path "$project_dir/.git" -o -path "$project_dir/target" -o -path "$project_dir/frontend/node_modules" -o -path "$project_dir/frontend/dist" \) -prune \
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

echo "Checking Rust formatting..."
cargo fmt --all -- --check

echo "Running Rust lints..."
cargo clippy --workspace --all-targets --locked -- -D warnings

echo "Running Rust tests..."
cargo test --workspace --all-targets --locked

echo "Building the frontend..."
cd "$project_dir/frontend"
npm run build

echo "All verification checks passed."

