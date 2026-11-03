# SPDX-License-Identifier: BSD-2-Clause
# The collector policy for UI apps (plan-wolkit section 4.5, measured in S0): never stop the world
# in the middle of an interaction. After the window has been idle for 100 ms and more than 1 MB was
# allocated since the last collection, collect now, while nobody is waiting. The scene lives in Rust,
# so the Crystal heap stays small and a full collection takes about 1 to 5 ms (S0 table).
module Wolkit
  module GCPolicy
    IDLE      = 100.milliseconds
    THRESHOLD = 1_000_000_u64

    @@last_total = 0_u64
    @@running = false

    def self.start : Nil
      return if @@running
      @@running = true
      @@last_total = GC.stats.total_bytes
      spawn(name: "wolkit-gc") do
        while @@running
          sleep IDLE
          collect_if_idle
        end
      end
    end

    def self.stop : Nil
      @@running = false
    end

    # Collect when the window has been quiet and enough was allocated. Returns true if it did.
    def self.collect_if_idle : Bool
      total = GC.stats.total_bytes
      return false unless Loop.idle_for >= IDLE && total - @@last_total > THRESHOLD
      collect
      true
    end

    # Collect now (a hidden window, a Memory Saver request).
    def self.collect : Nil
      GC.collect
      @@last_total = GC.stats.total_bytes
    end
  end
end
