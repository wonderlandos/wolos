# SPDX-License-Identifier: BSD-2-Clause
# Errors and the one place that catches everything. An app's handler, effect or cleanup that raises
# must not take the whole UI loop down, and nothing raised in Crystal may unwind into the engine. So
# every call from the runtime into app code goes through `Wolkit.guard`, which hands the exception to
# `Wolkit.on_error` (default: one line on STDERR; the dev tools show a banner instead).
module Wolkit
  # A programming mistake in the app (a signal written off the UI context, a view used outside a
  # mount, a bad argument to the DSL). Developer-facing text; never shown to a person.
  class UsageError < Exception
  end

  # A documented limit was reached (too many parts, a text too long). The message is a sentence
  # for the developer; the dev overlay shows it.
  class LimitError < Exception
  end

  @@on_error : Exception -> = ->(ex : Exception) do
    STDERR.puts "wolkit: #{ex.class}: #{ex.message}"
    nil
  end

  # Replace the handler for exceptions raised in app code. It must not raise.
  def self.on_error(&handler : Exception ->) : Nil
    @@on_error = handler
  end

  # The current handler.
  def self.on_error : Exception ->
    @@on_error
  end

  # Hand `ex` to the handler. Never raises.
  def self.report(ex : Exception) : Nil
    @@on_error.call(ex)
  rescue error
    STDERR.puts "wolkit: the error handler raised #{error.class} while handling #{ex.class}"
  end

  # Run app code. An exception goes to `report` and the caller carries on.
  def self.guard(& : ->) : Nil
    yield
  rescue ex
    report(ex)
  end

  # The block's value, or the exception it raised, so it can be raised again on another context.
  def self.capture(& : -> T) : T | Exception forall T
    yield
  rescue ex
    ex
  end
end
