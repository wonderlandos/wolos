# SPDX-License-Identifier: BSD-2-Clause
# Errors and the one place that catches everything. An app's handler, effect or cleanup that raises
# must not take the whole UI loop down, and nothing raised in Crystal may unwind into the engine. So
# every call from the runtime into app code goes through `Wolkit.guard`, which hands the exception to
# `Wolkit.on_error` (default: one line on STDERR; the dev tools show a banner instead). The catching
# itself is Cryswol's (`Wol::Callbacks`), the one boundary the repository allows for it.
require "../../../cryswol/src/cryswol"

module Wolkit
  # A programming mistake in the app (a signal written off the UI context, a view used outside a
  # mount, a bad argument to the DSL). Developer-facing text; never shown to a person.
  class UsageError < Exception
  end

  # A documented limit was reached (too many parts, a text too long). The message is a sentence
  # for the developer; the dev overlay shows it.
  class LimitError < Exception
  end

  @@on_diagnostic : Proc(UInt32, String, Nil) = ->(level : UInt32, message : String) do
    STDERR.puts "wolkit: #{%w(note warning error)[level]? || "note"}: #{message}"
    nil
  end

  # Replace the handler for exceptions raised in app code. It must not raise. This is Cryswol's own
  # `Wol::Loop.on_error`, so one handler serves callbacks from libwol and handlers of Wolkit.
  def self.on_error(&handler : Exception ->) : Nil
    Wol::Loop.on_error(&handler)
  end

  # The current handler.
  def self.on_error : Exception ->
    Wol::Loop.on_error
  end

  # Replace the handler for the engine's diagnostic events (level 0 note, 1 warning, 2 error). The
  # dev overlay shows them in a banner.
  def self.on_diagnostic(&handler : UInt32, String ->) : Nil
    @@on_diagnostic = handler
  end

  # An engine diagnostic arrived.
  def self.diagnostic(level : UInt32, message : String) : Nil
    @@on_diagnostic.call(level, message)
  end

  # Hand `ex` to the handler. Never raises.
  def self.report(ex : Exception) : Nil
    Wol::Callbacks.report(ex)
  end

  # Run app code. An exception goes to `report` and the caller carries on.
  def self.guard(& : ->) : Nil
    Wol::Callbacks.guard { yield }
  end

  # The block's value, or the exception it raised, so it can be raised again on another context.
  def self.capture(& : -> T) : T | Exception forall T
    Wol::Callbacks.capture { yield }
  end
end
