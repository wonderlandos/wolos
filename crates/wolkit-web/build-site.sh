#!/bin/bash
# SPDX-License-Identifier: BSD-2-Clause
# Builds the Wolkit web module and writes it where the site serves it (site/public/kit/):
# wolkit_web_bg.wasm and its wasm-bindgen glue wolkit_web.js. The loader (kit.js), the page and
# the Inter faces there are kept by hand; kit-build.json records the source commit, the toolchain
# and the sha256 of the two generated files. Prints the sizes (raw and gzip -9) against the budget.
# Usage: bash crates/wolkit-web/build-site.sh [SITE_DIR]   (default: ../site next to the checkout)
# Needs: rustup target wasm32-unknown-unknown, wasm-bindgen CLI equal to the crate (0.2.130).
set -euo pipefail
here=$(cd "$(dirname "$0")/../.." && pwd)
site=${1:-$here/../site}
out=$site/public/kit
target=${CARGO_TARGET_DIR:-$here/target}
want=$(sed -n 's/^wasm-bindgen = "=\(.*\)"/\1/p' "$here/crates/wolkit-web/Cargo.toml")
have=$(wasm-bindgen --version | awk '{print $2}')
[ "$want" = "$have" ] || { echo "build-site.sh: wasm-bindgen CLI $have, the crate pins $want (cargo install wasm-bindgen-cli --version $want)" >&2; exit 2; }
# No build-host path may reach the module (panic locations, debug strings): the checkout, the cargo
# registry and git sources, rustup and the cargo target dir are remapped to fixed names.
cargo_home=${CARGO_HOME:-$HOME/.cargo}
RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$cargo_home/registry/src=/cargo-registry --remap-path-prefix=$cargo_home/git/checkouts=/cargo-git --remap-path-prefix=${RUSTUP_HOME:-$HOME/.rustup}=/rustup --remap-path-prefix=$target=/target --remap-path-prefix=$here=/wolos" \
  cargo build --manifest-path "$here/Cargo.toml" -p wolkit-web --target wasm32-unknown-unknown --profile wasm
mkdir -p "$out"
wasm-bindgen --target web --no-typescript --remove-name-section --remove-producers-section \
  --out-dir "$out" "$target/wasm32-unknown-unknown/wasm/wolkit_web.wasm"
if grep -aqE '/(home|root|Users)/' "$out/wolkit_web_bg.wasm" "$out/wolkit_web.js"; then
  echo "build-site.sh: a build-host path is left in the module:" >&2
  grep -aoE '/(home|root|Users)/[A-Za-z0-9_./-]*' "$out/wolkit_web_bg.wasm" "$out/wolkit_web.js" | sort -u | head >&2
  exit 3
fi
# kit-build.json: what the generated files were built from; site/scripts/check-kit.mjs verifies it.
rev=$(git -C "$here" rev-parse HEAD)
git -C "$here" diff --quiet HEAD -- . 2>/dev/null || rev="$rev+uncommitted"
sha() { sha256sum "$1" | awk '{print $1}'; }
cat > "$out/kit-build.json" <<EOF
{
  "wolos": "$rev",
  "rustc": "$(rustc --version)",
  "wasm-bindgen": "$have",
  "wolkit_web_bg.wasm": "$(sha "$out/wolkit_web_bg.wasm")",
  "wolkit_web.js": "$(sha "$out/wolkit_web.js")"
}
EOF
budget=$((1536 * 1024))
total=0
for f in "$out/wolkit_web_bg.wasm" "$out/wolkit_web.js" "$out/kit.js" "$out/fonts/Inter-Regular.otf" "$out/fonts/Inter-SemiBold.otf"; do
  gz=$(gzip -9c "$f" | wc -c)
  total=$((total + gz))
  printf '%-28s %9d raw %9d gz\n' "$(basename "$f")" "$(wc -c < "$f")" "$gz"
done
printf 'total gz %d of budget %d (%s)\n' "$total" "$budget" "$([ $total -le $budget ] && echo within || echo OVER)"
