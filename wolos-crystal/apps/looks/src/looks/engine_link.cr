# SPDX-License-Identifier: GPL-2.0-or-later
# The palette engine of wol-hue (hue/, ADR 0031), required by path so the app and the command line
# tool share one engine: picture reading, colours, the eight palette modes, the sliders, the
# theme writer and the Base16 and colors.toml readers. Nothing here is copied.
require "../../../../../hue/src/hue/engine"
require "../../../../../hue/src/hue/imports"
require "../../../../../hue/src/hue/look"
