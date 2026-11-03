# SPDX-License-Identifier: BSD-2-Clause
# The part of Wolkit that needs no engine: the reactive runtime, the patch encoder, the mount and the
# DSL macros. `require "wolkit/core"` loads only this (specs, tools); `require "wolkit"` adds the
# engine binding, the loop and the app classes.
require "./guard"
require "./ui"
require "./reactive/source"
require "./reactive/scope"
require "./reactive/batch"
require "./reactive/effect"
require "./reactive/signal"
require "./reactive/computed"
require "./reactive/list"
require "./patch/wire"
require "./patch/writer"
require "./patch/describe"
require "./event"
require "./mount"
