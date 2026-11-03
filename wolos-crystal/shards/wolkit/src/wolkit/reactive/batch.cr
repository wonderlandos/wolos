# SPDX-License-Identifier: BSD-2-Clause
# Batching: writes inside a batch only queue the effects that depend on them; the queue flushes
# once, in creation order, when the outermost batch ends. So an effect runs at most once per flush
# and a parent's effect always runs before the effects of the rows it built (plan-wolkit section
# 3.2 "Batching and glitch freedom").
#
# The queue and the sort are preallocated: a flush of up to 64 effects allocates nothing.
module Wolkit
  module Batch
    # The most times one flush may re-run the queue before it is called a cycle.
    MAX_ROUNDS = 100

    @@queue = Array(Effect).new(64)
    @@depth = 0
    @@flushing = false

    # Queue `effect` unless it is already queued. Outside a batch it flushes at once.
    def self.schedule(effect : Effect) : Nil
      @@queue << effect
      flush if @@depth == 0 && !@@flushing
    end

    # How many effects wait (tests).
    def self.pending : Int32
      @@queue.size
    end

    # True inside `Wolkit.batch`.
    def self.active? : Bool
      @@depth > 0
    end

    # Run the block; flush when the outermost batch ends.
    def self.run(& : ->) : Nil
      @@depth += 1
      begin
        yield
      ensure
        @@depth -= 1
      end
      flush if @@depth == 0
    end

    # Run the queued effects, oldest first, until the queue is empty. An effect that writes a
    # signal queues more effects; they run in the next round of the same flush.
    def self.flush : Nil
      return if @@flushing
      @@flushing = true
      begin
        rounds = 0
        until @@queue.empty?
          rounds += 1
          if rounds > MAX_ROUNDS
            @@queue.clear
            raise Wolkit::UsageError.new("Effects keep triggering each other (a reactive cycle). Break it with `untracked` or a computed.")
          end
          sort_queue
          size = @@queue.size
          index = 0
          while index < size
            effect = @@queue[index]
            effect.run_queued
            index += 1
          end
          @@queue.delete_at(0, size)
        end
      ensure
        @@flushing = false
      end
    end

    # Insertion sort by effect order: the queue is short and nearly sorted.
    private def self.sort_queue : Nil
      index = 1
      while index < @@queue.size
        item = @@queue[index]
        prev = index - 1
        while prev >= 0 && @@queue[prev].order > item.order
          @@queue[prev + 1] = @@queue[prev]
          prev -= 1
        end
        @@queue[prev + 1] = item
        index += 1
      end
    end
  end

  # Group writes: the effects that depend on them run once, when the block ends.
  def self.batch(& : ->) : Nil
    Batch.run { yield }
  end
end
