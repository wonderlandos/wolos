#!/bin/sh
# Build the repo Rust workspace (cargo build --release) and install WolOS binaries into DEST.
# Usage: image/build-rust.sh DEST   (run by the wolos-system PKGBUILD; needs cargo, pkgconf, GTK4 deps)
# Env: CARGO_TARGET_DIR (default target/), WOLOS_VENDOR (vendored crates: build offline).
set -eu
DEST=${1:?DEST}
REPO=$(cd "$(dirname "$0")/.." && pwd)
command -v cargo >/dev/null || { echo "build-rust.sh: cargo missing (pacman -S rust)" >&2; exit 1; }
cd "$REPO"
[ -f Cargo.toml ] || { echo "build-rust.sh: no workspace Cargo.toml in $REPO" >&2; exit 1; }
# Real shell surfaces (layer-shell + session-lock) when the C library is present (gtk4-layer-shell).
F=; pkg-config --exists gtk4-layer-shell-0 && F="--features shell/layer-shell"
if [ -n "${WOLOS_VENDOR:-}" ]; then
  [ -d "$WOLOS_VENDOR" ] || { echo "build-rust.sh: WOLOS_VENDOR=$WOLOS_VENDOR missing" >&2; exit 1; }
  F="$F --offline --config 'source.crates-io.replace-with=\"vendored\"' --config 'source.vendored.directory=\"$WOLOS_VENDOR\"'"
fi
# shellcheck disable=SC2086
# Faster link and cached rebuilds when the tools are installed (CI, developer machines).
command -v mold >/dev/null && command -v clang >/dev/null && export RUSTFLAGS="${RUSTFLAGS:-} -C linker=clang -C link-arg=-fuse-ld=mold"
command -v sccache >/dev/null && export RUSTC_WRAPPER=sccache
eval cargo build --release --workspace --locked $F
B=${CARGO_TARGET_DIR:-$REPO/target}/release
mkdir -p "$DEST/usr/bin" "$DEST/usr/lib"
for b in wold wol-admind wol wol-login wol-run wol-realtime wol-shell wol-spotlight wol-control-center wol-lock wol-settings wol-loginwindow wol-setup wol-installer wol-window-shot wol-app wol-store wol-flatpak-consent; do
  [ -x "$B/$b" ] || { echo "build-rust.sh: binary $b not produced" >&2; exit 1; }
  install -m 755 "$B/$b" "$DEST/usr/bin/$b"
done
# pacman for the sealed system: first on PATH, runs the real /usr/bin/pacman (layers, ADR 0016).
[ -x "$B/pacman" ] || { echo "build-rust.sh: binary pacman not produced" >&2; exit 1; }
install -Dm755 "$B/pacman" "$DEST/usr/local/bin/pacman"
[ -x "$B/wol-authcheck" ] || { echo "build-rust.sh: binary wol-authcheck not produced" >&2; exit 1; }
# setuid root: checks a password for wol-login and wol-admind (fakeroot in makepkg records root).
install -m 4755 "$B/wol-authcheck" "$DEST/usr/lib/wol-authcheck"
# Root-only session runner for wol-login (PAM session, then the user's login shell).
[ -x "$B/wol-session-pam" ] || { echo "build-rust.sh: binary wol-session-pam not produced" >&2; exit 1; }
install -m 755 "$B/wol-session-pam" "$DEST/usr/lib/wol-session-pam"
