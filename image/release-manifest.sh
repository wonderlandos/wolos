#!/usr/bin/env bash
# Release manifest for an image: ISO SNAPSHOT REPO_REVISION BASE_URL -> JSON on stdout.
# Read by updates and the download page: which Arch Linux Archive date, which [wolos] repo revision,
# where the image is and its sha256.
set -euo pipefail
ISO=${1:?ISO}; SNAP=${2:?SNAPSHOT}; REV=${3:?REVISION}; URL=${4:?BASE_URL}
sum=$(sha256sum "$ISO" | cut -d' ' -f1)
size=$(stat -c %s "$ISO")
printf '{"snapshot":"%s","repo_revision":"%s","image":"%s/%s","sha256":"%s","size":%s}\n' \
  "$SNAP" "$REV" "${URL%/}" "$(basename "$ISO")" "$sum" "$size"
