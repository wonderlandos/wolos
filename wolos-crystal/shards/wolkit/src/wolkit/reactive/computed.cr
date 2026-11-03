# SPDX-License-Identifier: BSD-2-Clause
# Computed(T): a lazy derived value. It recomputes on the first read after a source changed and
# caches the result; it is both a Source (effects read it) and an Observer (it reads signals).
module Wolkit
  # What a computed's link tells when a source changed.
  module Invalidatable
    abstract def invalidate : Nil
  end

  # The observer half of a computed (a class cannot inherit both Source and Observer).
  class ComputedLink < Observer
    property owner : Invalidatable? = nil

    def mark : Nil
      @owner.try &.invalidate
    end

    def relink : Nil
      unlink_sources
    end
  end

  class Computed(T) < Source
    include Invalidatable
    include Readable(T)

    @cache : T? = nil
    @dirty = true
    @link = ComputedLink.new

    def initialize(&@compute : -> T)
      @link.owner = self
    end

    # The value, recomputed when a source changed since the last read. Records a dependency.
    def value : T
      track
      compute_now
    end

    def peek : T
      compute_now
    end

    # A source changed: forget the cache and tell the readers.
    def invalidate : Nil
      return if @dirty
      @dirty = true
      notify
    end

    private def compute_now : T
      if @dirty
        @link.relink
        fresh = Tracker.with(@link) { @compute.call }
        @cache = fresh
        @dirty = false
        fresh
      else
        @cache.as(T)
      end
    end

    forward_missing_to value
  end

  # A derived value: `computed { @first.value + " " + @last.value }`.
  def self.computed(&block : -> T) : Computed(T) forall T
    Computed(T).new(&block)
  end
end
