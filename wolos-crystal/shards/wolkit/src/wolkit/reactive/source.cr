# SPDX-License-Identifier: BSD-2-Clause
# The dependency graph of the reactive runtime: a Source (signal, computed, list) is read by
# Observers (effects, computeds). Reading inside an observer records the edge; a write marks the
# observers, and the batch decides when they run (plan-wolkit section 3.2).
#
# Concept: fine-grained reactivity. Why: a state change must cost O(dependents), not O(tree).
# How: `Tracker.current` is the observer whose body is running; `Source#track` links it to the
# source being read; `Observer#mark` is called on a write and either queues an effect or
# invalidates a computed and passes the mark on.
module Wolkit
  # Something an observer can depend on.
  abstract class Source
    @observers = [] of Observer

    # Record that the running observer (if any) read this source.
    protected def track : Nil
      observer = Tracker.current
      return unless observer
      return if @observers.includes?(observer)
      @observers << observer
      observer.sources << self
    end

    # Mark every observer. The marks only queue work; the batch runs it after the loop, because
    # a re-running observer changes this very list.
    protected def notify : Nil
      Batch.run { @observers.each &.mark }
    end

    # Forget `observer` (it re-runs and re-reads, or it was disposed).
    def forget(observer : Observer) : Nil
      @observers.delete(observer)
    end

    # How many observers read this source (for tests and the inspector).
    def observer_count : Int32
      @observers.size
    end
  end

  # Something that reads sources and must hear about their changes.
  abstract class Observer
    getter sources = [] of Source

    # A source this observer read has changed.
    abstract def mark : Nil

    # Drop every edge before a re-run, so the next run records exactly what it reads.
    protected def unlink_sources : Nil
      @sources.each &.forget(self)
      @sources.clear
    end
  end

  # Which observer is running right now.
  module Tracker
    @@current : Observer? = nil

    def self.current : Observer?
      @@current
    end

    # Run the block with `observer` as the reader.
    def self.with(observer : Observer?, & : -> T) : T forall T
      previous = @@current
      @@current = observer
      begin
        yield
      ensure
        @@current = previous
      end
    end
  end

  # Run the block without recording what it reads.
  def self.untracked(& : -> T) : T forall T
    Tracker.with(nil) { yield }
  end

  # A value an effect can read and depend on: a signal, a computed or a constant.
  module Readable(T)
    # The value; inside an effect this records a dependency.
    abstract def value : T

    # The value without recording a dependency.
    abstract def peek : T

    def to_s(io : IO) : Nil
      value.to_s(io)
    end

    def inspect(io : IO) : Nil
      io << "#<" << self.class.name << ' ' << peek.inspect << '>'
    end
  end

  # A value that can also be written.
  module Writable(T)
    abstract def value=(value : T) : T
  end

  # Unwrap a readable inside an effect (tracked) or pass a plain value through.
  def self.read(readable : Readable(T)) : T forall T
    readable.value
  end

  def self.read(value : T) : T forall T
    value
  end
end
