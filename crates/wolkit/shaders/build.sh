#!/bin/bash
# SPDX-License-Identifier: BSD-2-Clause
# Compiles the uber-pipeline shaders to the SPIR-V that src/gpu/pipelines.rs embeds, and records
# FNV-1a 64 of every source and output in spv.lock. tests/gpu_shaders.rs fails when a source no
# longer matches the lock (edit, then rerun this script on a box with glslang: the forge has it).
# Usage: bash shaders/build.sh
set -euo pipefail
cd "$(dirname "$0")"
command -v glslangValidator >/dev/null || { echo "build.sh: glslangValidator is not installed (pacman -S glslang)." >&2; exit 2; }
fnv() { python3 -I -c 'import sys
h = 0xcbf29ce484222325
for b in open(sys.argv[1], "rb").read():
    h = ((h ^ b) * 0x100000001b3) & 0xffffffffffffffff
print("%016x" % h)' "$1"; }
for s in ui.vert ui.frag; do
  glslangValidator -V --target-env vulkan1.0 -o "$s.spv" "$s" >/dev/null
  spirv-val "$s.spv"
done
{
  echo "# Written by shaders/build.sh; checked by tests/gpu_shaders.rs. source fnv1a64, spirv fnv1a64."
  echo "glslang $(glslangValidator --version | sed -n 's/^Glslang Version: //p' | head -1)"
  for s in ui.vert ui.frag; do echo "$s $(fnv "$s") $s.spv $(fnv "$s.spv")"; done
} > spv.lock
cat spv.lock
