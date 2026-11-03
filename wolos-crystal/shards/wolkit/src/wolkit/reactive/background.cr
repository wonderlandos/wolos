# SPDX-License-Identifier: BSD-2-Clause
# Background work: a small pool of threads (a Fiber::ExecutionContext::Parallel) for work that would
# block the UI, and the way back to the UI context. `Wolkit.background { work }` runs the block on the
# pool and calls `then` on the UI context with the result. Signals are never touched from the pool.
# A block that needs libwol (a file panel, a share sheet) opens its own context with `Wol.background`.
module Wolkit
  module Background
    WORKERS = 4

    @@pool : Fiber::ExecutionContext::Parallel? = nil

    # The pool, made on first use.
    def self.pool : Fiber::ExecutionContext::Parallel
      @@pool ||= Fiber::ExecutionContext::Parallel.new("wolkit-background", WORKERS)
    end

    # Run `work` on the pool; `done` runs on the UI context, in a batch, with the value or the
    # exception it raised.
    def self.run(work : -> T, done : T | Exception ->) : Nil forall T
      pool.spawn(name: "wolkit-work") do
        result = Wolkit.capture { work.call }
        UI.post { done.call(result) }
      end
    end
  end
end
