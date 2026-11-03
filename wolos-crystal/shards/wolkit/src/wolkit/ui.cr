# SPDX-License-Identifier: BSD-2-Clause
# The UI context: signals, effects and views are used on one execution context only (ADR 0036,
# docs/modules/cryswol.md "Threads"). `UI.claim` records it; `UI.post` hands work from another
# thread to it. Why a channel: Crystal channels work across execution contexts, and the loop fiber
# can wait on one without a second file descriptor.
module Wolkit
  module UI
    # Work posted from other threads waits here until the UI context runs it.
    POSTS = Channel(Proc(Nil)).new(1024)

    @@context : Fiber::ExecutionContext? = nil

    # Record the calling context as the UI context (the loop and the test harness call this).
    def self.claim : Nil
      @@context = Fiber::ExecutionContext.current
    end

    # Forget the UI context (specs).
    def self.release : Nil
      @@context = nil
    end

    # True on the UI context, or before any context was claimed.
    def self.ui? : Bool
      context = @@context
      context.nil? || context.same?(Fiber::ExecutionContext.current)
    end

    # Dev builds: a signal written off the UI context raises. Release builds skip the check.
    def self.check_context(what : String) : Nil
      {% if flag?(:dev) %}
        return if ui?
        raise Wolkit::UsageError.new("#{what} was called off the UI context. Use `set_later`, or `Wolkit::UI.post { ... }`.")
      {% end %}
    end

    # Run the block on the UI context, in a batch, at the next loop turn. Safe from any thread.
    def self.post(&block : ->) : Nil
      POSTS.send(block)
    end

    # Run everything posted so far, each in its own batch (the loop calls this after waking;
    # specs call it directly). Returns how many ran.
    def self.run_posted : Int32
      count = 0
      loop do
        select
        when work = POSTS.receive
          Wolkit.batch { Wolkit.guard { work.call } }
          count += 1
        else
          break
        end
      end
      count
    end
  end
end
