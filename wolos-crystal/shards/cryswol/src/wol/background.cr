# SPDX-License-Identifier: BSD-2-Clause
# Background work for the verbs that block for seconds or minutes (panels, share, message send;
# plan-cr1 §3 rule 9). wol.h allows one thread at a time per context, so the work runs on its own
# thread with its own context for the same app id, never on the loop's context. Crystal 1.22 has
# execution contexts without flags ("S0 results"), and shard.yml requires it.
require "./lib_wol"
require "./error"
require "./context"
require "./callbacks"

module Wol
  # Run `block` on its own thread with a fresh context for `app_id`, and return its result
  # to the calling fiber, which waits without blocking other fibers. An exception in the block
  # is raised here. The context is closed when the block returns.
  #
  # ```
  # file = Wol.background(ctx.app_id) { |bg| Wol::Panel.open(bg, "Open a picture") }
  # ```
  def self.background(app_id : ::String, &block : Context -> T) : T forall T
    done = Channel(T | Exception).new(1)
    Fiber::ExecutionContext::Isolated.new("cryswol-background") do
      done.send(Callbacks.capture { Context.open(app_id) { |ctx| block.call(ctx) } })
    end
    result = done.receive
    raise result if result.is_a?(Exception)
    result
  end
end
