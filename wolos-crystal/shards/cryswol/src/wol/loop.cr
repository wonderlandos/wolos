# SPDX-License-Identifier: BSD-2-Clause
# The event loop (wol.h "Event loop", plan-cr1 §3 rule 10 and "S0 results").
#
# Concept: libwol queues what arrives and makes its wake fd readable; `dispatch` runs the
# callbacks. Why a fiber: a Crystal app has other fibers (timers, sockets), so the loop must
# block the fiber, not the thread. How: the loop does a fiber-blocking read on a dup of the wake
# fd, then calls `dispatch(0)`. libwol's dispatch drains the pipe before the queue
# (crates/wol-abi/src/context.rs, `dispatch`), so no wake-up is lost; an extra byte only causes one
# empty dispatch.
#
# A callback that raises never unwinds into libwol: `Callbacks.invoke` hands the exception to
# `Loop.on_error`, and the loop keeps running.
require "./lib_wol"
require "./error"
require "./context"
require "./callbacks"

module Wol
  module Loop
    @@on_error : Exception -> = ->(ex : Exception) do
      STDERR.puts "cryswol: a callback raised #{ex.class}: #{ex.message}"
      nil
    end

    # Replace the handler for exceptions raised in callbacks and in dispatch. The default
    # prints one line to STDERR. The handler must not raise; if it does, that is printed too.
    def self.on_error=(handler : Exception ->) : Exception ->
      @@on_error = handler
    end

    # Same as `on_error=`, with a block.
    def self.on_error(&handler : Exception ->) : Nil
      @@on_error = handler
    end

    # The current handler.
    def self.on_error : Exception ->
      @@on_error
    end

    # Hand `ex` to the error handler. Never raises.
    def self.report(ex : Exception) : Nil
      Callbacks.report(ex)
    end

    # Run the callbacks of `context` until `stop`, the context closes, or `timeout` passes.
    # Blocks the calling fiber only.
    def self.run(context : Context, timeout : Time::Span? = nil) : Nil
      run(context, timeout) { false }
    end

    # Run until the block returns true (checked after every dispatch), `stop`, the context
    # closes, or `timeout` passes. Returns true when the block said so.
    def self.run(context : Context, timeout : Time::Span? = nil, & : -> Bool) : Bool
      raise UsageError.new("Loop.run called from inside a callback") if context.dispatching?
      raise UsageError.new("Loop.run is already running for this context") if context.running?
      context.running = true
      deadline = timeout.try { |t| Time.instant + t }
      buf = Bytes.new(256)
      begin
        loop do
          step(context)
          return true if yield
          return false if context.closed? || !context.running?
          io = context.wake_io
          if deadline
            left = deadline - Time.instant
            return false if left <= Time::Span.zero
            io.read_timeout = left
          else
            io.read_timeout = nil
          end
          begin
            # 0 means libwol's write end is gone: nothing can arrive any more.
            return false if io.read(buf) == 0
          rescue IO::TimeoutError
            step(context)
            return yield
          rescue IO::Error
            # `stop` or `Context#close` closed the dup to wake us; the checks above decide.
          end
        end
      ensure
        context.running = false
      end
    end

    # Make `run` return for `context`. Works from a callback or from another fiber.
    def self.stop(context : Context) : Nil
      context.running = false
      context.close_wake
    end

    # One `dispatch(0)`; its errors go to `on_error` so the loop keeps running.
    private def self.step(context : Context) : Nil
      return if context.closed?
      context.dispatch(0)
    rescue ex : Wol::Error
      report(ex)
    end
  end
end
