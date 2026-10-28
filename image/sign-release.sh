#!/usr/bin/env bash
# sign-release.sh FILE KEY.pem: FILE.sig = raw Ed25519 signature of FILE's bytes (64 bytes), the
# form `wol update` verifies release.json with (the package keys, wol_core::package::verify_index).
set -euo pipefail
F=${1:?FILE}; K=${2:?KEY.pem}
openssl pkeyutl -sign -rawin -inkey "$K" -in "$F" -out "$F.sig"
[ "$(stat -c %s "$F.sig")" = 64 ] || { echo "sign-release.sh: not an Ed25519 key" >&2; exit 1; }
