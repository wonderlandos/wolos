#!/bin/sh
# Build the repo Rust workspace (cargo build --release) and install WolOS binaries into DEST.
# Usage: image/build-rust.sh DEST   Runs on FreeBSD (pkg install rust pkgconf + GTK/wayland deps of the shell crates).
# Env: CARGO_TARGET_DIR (default /work/target/wolos).
set -eu
DEST=${1:?DEST}
REPO=$(cd "$(dirname "$0")/.." && pwd)
[ "$(uname -s)" = FreeBSD ] || { echo "build-rust.sh: run on FreeBSD" >&2; exit 1; }
command -v cargo >/dev/null || { echo "build-rust.sh: cargo missing (pkg install rust)" >&2; exit 1; }
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-/work/target/wolos}
cd "$REPO"
[ -f Cargo.toml ] || { echo "build-rust.sh: no workspace Cargo.toml in $REPO" >&2; exit 1; }
# Real shell surfaces (layer-shell + session-lock) when the C library is present (pkg gtk4-layer-shell).
F=; pkg-config --exists gtk4-layer-shell-0 && F="--features shell/layer-shell"
# WOLOS_VENDOR: vendored crates (image/vm/build-in-vm.sh vendors on the host): build offline, no downloads.
if [ -n "${WOLOS_VENDOR:-}" ]; then
  [ -d "$WOLOS_VENDOR" ] || { echo "build-rust.sh: WOLOS_VENDOR=$WOLOS_VENDOR missing" >&2; exit 1; }
  F="$F --offline --config 'source.crates-io.replace-with=\"vendored\"' --config 'source.vendored.directory=\"$WOLOS_VENDOR\"'"
fi
# shellcheck disable=SC2086
eval cargo build --release --workspace --locked $F
B=$CARGO_TARGET_DIR/release
mkdir -p "$DEST/usr/local/bin" "$DEST/usr/local/libexec"
for b in wold wol-admind wol wol-login wol-run wol-realtime wol-shell wol-spotlight wol-control-center wol-lock wol-settings wol-loginwindow wol-setup wol-installer wol-window-shot wol-app wol-store; do
  [ -x "$B/$b" ] || { echo "build-rust.sh: binary $b not produced" >&2; exit 1; }
  install -m 755 "$B/$b" "$DEST/usr/local/bin/$b"
done
[ -x "$B/wol-authcheck" ] || { echo "build-rust.sh: binary wol-authcheck not produced" >&2; exit 1; }
install -o root -g wheel -m 4555 "$B/wol-authcheck" "$DEST/usr/local/libexec/wol-authcheck"
