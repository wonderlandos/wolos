# SPDX-License-Identifier: BSD-2-Clause
# Effect: a tracked side effect. The body runs once at creation and again whenever a source it read
# changes (queued by `Batch`, so at most once per flush). A body that read nothing is dropped after
# the first run: it can never run again, so a binding to a literal costs nothing afterwards.
# `on_cleanup` inside the body runs before each re-run and at dispose.
module Wolkit
  class Effect < Observer
    include Disposable

    @@counter = 0_u64

    # Creation number; the batch runs lower numbers first (a parent before its children).
    getter order : UInt64
    getter? disposed : Bool = false
    @queued = false
    @cleanups = [] of Proc(Nil)
    @body : Proc(Nil)

    # Make the effect and run it once. It belongs to `Scope.current`.
    def initialize(&@body : ->)
      @@counter += 1
      @order = @@counter
      Scope.current.add(self)
      execute
    end

    # A source changed: queue a re-run.
    def mark : Nil
      return if @disposed || @queued
      @queued = true
      Batch.schedule(self)
    end

    # Called by the batch.
    protected def run_queued : Nil
      @queued = false
      execute unless @disposed
    end

    # Run `block` before the next re-run and when the effect is disposed.
    def on_cleanup(&block : -> Nil) : Nil
      @cleanups << block
    end

    # True when the last run read at least one source.
    def dependent? : Bool
      !@sources.empty?
    end

    def dispose : Nil
      return if @disposed
      @disposed = true
      unlink_sources
      run_cleanups
    end

    private def execute : Nil
      run_cleanups
      unlink_sources
      Tracker.with(self) do
        Wolkit.guard { @body.call }
      end
      # Nothing to depend on: it will never run again, so let it go.
      dispose if @sources.empty?
    end

    private def run_cleanups : Nil
      until @cleanups.empty?
        cleanup = @cleanups.pop
        Wolkit.guard { cleanup.call }
      end
    end
  end

  # Run `block` now and again whenever a signal it read changes. Returns the effect (it is
  # disposed with the current scope; `dispose` it earlier to stop it).
  def self.effect(&block : ->) : Effect
    Effect.new(&block)
  end

  # Inside an effect: run `block` before its next re-run and when it is disposed. Elsewhere: when
  # the current scope is disposed.
  def self.on_cleanup(&block : -> Nil) : Nil
    case observer = Tracker.current
    when Effect then observer.on_cleanup(&block)
    else             Scope.current.on_cleanup(&block)
    end
  end
end
