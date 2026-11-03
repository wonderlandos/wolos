# SPDX-License-Identifier: BSD-2-Clause
# Timers: `every(1.second) { }` and `after(300.milliseconds) { }`. A timer belongs to the scope it
# was made in and stops when that scope is disposed, so a row or a screen that goes away takes its
# timers with it. The block runs on the UI context, in a batch, so the effects it triggers run once
# and the window gets one patch.
module Wolkit
  class Timer
    include Disposable

    getter? repeating : Bool
    getter? cancelled : Bool = false
    @stop = Channel(Nil).new

    def initialize(@span : Time::Span, @repeating : Bool, @block : Proc(Nil))
      Scope.current.add(self)
      spawn(name: "wolkit-timer") { run }
    end

    # Stop the timer; a block that has not run yet will not.
    def dispose : Nil
      return if @cancelled
      @cancelled = true
      @stop.close
    end

    private def run : Nil
      until @cancelled
        select
        when @stop.receive?
          break
        when timeout(@span)
          break if @cancelled
          Wolkit.batch { Wolkit.guard { @block.call } }
          Frame.request
          break unless @repeating
        end
      end
    end
  end

  # Run the block every `span` until the scope ends.
  def self.every(span : Time::Span, &block : ->) : Timer
    Timer.new(span, true, block)
  end

  # Run the block once after `span`, unless the scope ends first.
  def self.after(span : Time::Span, &block : ->) : Timer
    Timer.new(span, false, block)
  end

  module DSL
    # Run the block every `span` until this view goes away.
    def every(span : Time::Span, &block : ->) : Timer
      Wolkit.every(span, &block)
    end

    # Run the block once after `span`, unless this view goes away first.
    def after(span : Time::Span, &block : ->) : Timer
      Wolkit.after(span, &block)
    end
  end
end
