#!/usr/bin/env bash
# Builds every logic component for wasm32-wasip2 in release and places each at
# target/guests/<name>.wasm under the workspace root, where the host's tests and the
# fly-through load them (ADR-0010). <name> is the directory name of a crate under
# crates/vxe-script-wasm/guests/. The components are build output, never committed.
#
# Needs the target: rustup target add wasm32-wasip2
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
target_dir="${CARGO_TARGET_DIR:-$root/target}"
out="$root/target/guests"
mkdir -p "$out"

# build <package> <output name> [cargo args...]
build() {
  local package="$1" name="$2"
  shift 2
  cargo build --manifest-path "$root/Cargo.toml" -p "$package" \
    --target wasm32-wasip2 --release "$@"
  cp "$target_dir/wasm32-wasip2/release/${package//-/_}.wasm" "$out/$name.wasm"
  echo "built $out/$name.wasm"
}

for dir in "$root"/crates/vxe-script-wasm/guests/*/; do
  package="$(sed -n 's/^name *= *"\(.*\)"/\1/p' "$dir/Cargo.toml" | head -n 1)"
  build "$package" "$(basename "$dir")"
done
