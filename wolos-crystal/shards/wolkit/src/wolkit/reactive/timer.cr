# SPDX-License-Identifier: BSD-2-Clause
# Timers: `every(1.second) { }` and `after(300.milliseconds) { }`, and the helpers built on them:
# `debounce`, `throttle` and `interval`. A timer belongs to the scope it was made in and stops when
# that scope is disposed, so a row or a screen that goes away takes its timers with it. The block
# runs on the UI context, in a batch, so the effects it triggers run once and the window gets one
# patch.
#
# Concept: a spec must not wait for a clock. `Wolkit::Timer.manual!` stops timers from sleeping:
# they wait in a queue until `Wolkit::Timer.advance(span)` moves the manual clock, which runs every
# block that came due, oldest first (a repeating timer can run several times in one advance). Why:
# a debounced search field is tested in microseconds and the same way on every machine.
module Wolkit
  class Timer
    include Disposable

    getter? repeating : Bool
    getter? cancelled : Bool = false
    # The manual clock's time this timer runs next (manual mode only).
    getter due : Time::Span = Time::Span.zero
    @stop = Channel(Nil).new

    @@manual = false
    @@now = Time::Span.zero
    @@queue = [] of Timer
    @@made = 0_u64
    @order = 0_u64

    def initialize(@span : Time::Span, @repeating : Bool, @block : Proc(Nil))
      Scope.current.add(self)
      @order = (@@made += 1)
      if @@manual
        @due = @@now + @span
        @@queue << self
      else
        spawn(name: "wolkit-timer") { run }
      end
    end

    # Stop the timer; a block that has not run yet will not.
    def dispose : Nil
      return if @cancelled
      @cancelled = true
      @stop.close
      @@queue.delete(self) if @@manual
    end

    private def run : Nil
      until @cancelled
        select
        when @stop.receive?
          break
        when timeout(@span)
          break if @cancelled
          fire
          break unless @repeating
        end
      end
    end

    protected def fire : Nil
      Wolkit.batch { Wolkit.guard { @block.call } }
      Frame.request
    end

    # Timers made from now on wait for `advance` instead of the real clock (specs).
    def self.manual! : Nil
      @@manual = true
      @@now = Time::Span.zero
      @@queue.clear
    end

    # Back to the real clock; timers still queued are dropped.
    def self.real! : Nil
      @@manual = false
      @@queue.clear
    end

    def self.manual? : Bool
      @@manual
    end

    # The manual clock.
    def self.now : Time::Span
      @@now
    end

    # Timers waiting on the manual clock.
    def self.pending : Int32
      @@queue.size
    end

    # Move the manual clock by `span` and run what came due, in time order.
    def self.advance(span : Time::Span) : Nil
      raise UsageError.new("Timer.advance needs the manual clock: call Wolkit::Timer.manual! first.") unless @@manual
      target = @@now + span
      loop do
        timer = @@queue.select { |t| t.due <= target }.min_by? { |t| {t.due, t.order} }
        break unless timer
        @@now = timer.due
        if timer.repeating?
          timer.step
          timer.fire
        else
          @@queue.delete(timer)
          timer.fire
          timer.dispose
        end
      end
      @@now = target
    end

    protected def order : UInt64
      @order
    end

    protected def step : Nil
      @due += @span.zero? ? 1.nanosecond : @span
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

  # One timer that is started again and again (debounce, throttle): each start stops the one before.
  # The timer lives in a scope of its own under the scope that made the Rearm, so restarting does
  # not pile up stopped timers, and the owner's end stops it.
  class Rearm
    @owner : Scope
    @scope : Scope? = nil

    def initialize
      @owner = Scope.current
    end

    def start(span : Time::Span, repeating : Bool, &block : -> Nil) : Nil
      cancel
      scope = Scope.new(@owner)
      @scope = scope
      scope.run { Timer.new(span, repeating, block) }
    end

    def cancel : Nil
      @scope.try &.dispose
      @scope = nil
    end

    def active? : Bool
      !@scope.nil?
    end
  end

  # A signal that follows `source` once it has stopped changing for `span` (a search box that asks
  # only when the person pauses). It starts with the source's value.
  def self.debounce(source : Readable(T), span : Time::Span) : Signal(T) forall T
    follower = Signal(T).new(source.peek)
    timer = Rearm.new
    first = true
    Wolkit.effect do
      value = source.value
      if first
        first = false
      else
        timer.start(span, false) do
          timer.cancel
          follower.value = value
          nil
        end
      end
    end
    follower
  end

  # A signal that follows `source` at most once per `span`: the first change at once, then the
  # latest value at the end of each span while it keeps changing (a scroll position, a slider).
  def self.throttle(source : Readable(T), span : Time::Span) : Signal(T) forall T
    follower = Signal(T).new(source.peek)
    timer = Rearm.new
    latest = source.peek
    dirty = false
    first = true
    Wolkit.effect do
      latest = source.value
      if first
        first = false
      elsif timer.active?
        dirty = true
      else
        follower.value = latest
        dirty = false
        timer.start(span, true) do
          if dirty
            follower.value = latest
            dirty = false
          else
            timer.cancel
          end
          nil
        end
      end
    end
    follower
  end

  # A counter that goes up by one every `span` until the scope ends: `text "#{interval(1.second)}"`.
  def self.interval(span : Time::Span) : Signal(Int64)
    ticks = Signal(Int64).new(0_i64)
    Timer.new(span, true, -> { ticks.value = ticks.peek + 1; nil })
    ticks
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

    # `source` once it has been still for `span`.
    def debounce(source : Readable(T), span : Time::Span) : Signal(T) forall T
      Wolkit.debounce(source, span)
    end

    # `source` at most once per `span`.
    def throttle(source : Readable(T), span : Time::Span) : Signal(T) forall T
      Wolkit.throttle(source, span)
    end

    # A tick count that goes up every `span`.
    def interval(span : Time::Span) : Signal(Int64)
      Wolkit.interval(span)
    end
  end
end
