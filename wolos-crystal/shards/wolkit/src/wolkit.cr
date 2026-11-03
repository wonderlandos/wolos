# SPDX-License-Identifier: BSD-2-Clause
# Entry point for `require "wolkit"`: Wolkit, the WolOS UI kit for Crystal (ADR 0039). The DSL, the
# reactive runtime and the app classes come from `wolkit/core`; this file adds the engine binding,
# the loop and `Wolkit.run`. Build it on Cryswol (`Wol::Handle`).
require "./wolkit/core"
require "./wolkit/runtime"
require "./wolkit/services"
require "./wolkit/loop"
require "./wolkit/gc_policy"
require "./wolkit/kit"
require "./wolkit/dev"
