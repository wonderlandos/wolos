# SPDX-License-Identifier: BSD-2-Clause
# Cryswol: the WolOS app framework for Crystal, on libwol (ADR 0036, docs/modules/cryswol.md).
# `LibWol` is the generated C layer; `Wol` is the layer apps use. Every file in src/wol/ is part of
# it (one glob, so a new verb file needs no edit here). The GPL opt-in parts live in subdirectories
# the glob does not reach: `require "cryswol/pkg"`, `require "cryswol/realtime"`.
require "json"
require "./wol/lib_wol"
require "./wol/*"

# The WolOS app API. Start with `Wol::Context.open` and `Wol::Loop.run`.
module Wol
end
