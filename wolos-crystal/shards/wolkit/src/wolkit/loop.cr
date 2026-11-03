# SPDX-License-Identifier: BSD-2-Clause
# Wolkit::Loop: the one fiber that drives the engine (plan-wolkit section 4.2). It waits for the
# engine's descriptor to become readable (fiber-blocking, so timers and other fibers run), calls
# `dispatch(0)`, drains the event stream, runs the handlers in ONE batch, flushes the effects, and
# sends one patch per window. ADR 0036's "one event loop" holds: one scheduler, two descriptors
# (this one and Cryswol's wake fd, which `Wol::Loop` waits on in its own fiber).
#
# Concept: handlers run on this fiber, never inside the engine, so a Crystal exception can never
# unwind through C. Why one batch per turn: the effects that depend on the changes run once, in
# order, and each window gets a single `wolkit_window_apply`.
require "./runtime"

module Wolkit
  module Loop
    @@activity = Time.instant
    # One pending wake-up is enough: the loop turns once and handles everything that is there.
    SIGNAL = Channel(Nil).new(1)
    # Posted in place of work to end the pump fiber (the posts channel itself outlives the loop).
    STOP_PUMP = -> { nil }

    # Make the loop take a turn (the engine's descriptor does it for engine events; `Runtime.quit` and
    # the headless tests use this one).
    def self.wake : Nil
      select
      when SIGNAL.send(nil)
      else
      end
    end

    # Run until the last window closes or the engine says quit.
    def self.run(engine : FFI::Engine) : Nil
      UI.claim
      Frame.auto = true
      Runtime.on_quit = -> { wake }
      wake_io = engine.wake_io
      spawn(name: "wolkit-engine-fd") { watch(wake_io) }
      pump = spawn(name: "wolkit-posts") { pump_posts }
      GCPolicy.start
      begin
        until Runtime.quit?
          turn(engine)
          break if Runtime.quit?
          SIGNAL.receive
        end
      ensure
        Frame.auto = false
        Runtime.on_quit = nil
        GCPolicy.stop
        wake_io.close
        UI::POSTS.send(STOP_PUMP) unless pump.dead?
      end
    end

    # One turn: dispatch, drain the events into the handlers, flush the frame.
    def self.turn(engine : FFI::Engine) : Nil
      flags = engine.dispatch(0)
      seen = 0
      Wolkit.batch do
        loop do
          count = 0
          engine.read_events.each do |event|
            count += 1
            Runtime.dispatch(event)
          end
          seen += count
          break if count == 0
        end
        UI.run_posted
      end
      @@activity = Time.instant if seen > 0
      Frame.flush_all
      Runtime.quit if flags & LibWolkit::WOLKIT_DISPATCH_QUIT != 0
    end

    # How long nothing happened.
    def self.idle_for : Time::Span
      Time.instant - @@activity
    end

    # Wait on the engine's descriptor (a duplicate Crystal owns) and wake the loop whenever it is
    # readable. The engine's descriptor is a self-pipe: reading it only consumes the wake-up bytes, and
    # `dispatch(0)` runs what is queued whether or not a byte was there, so no wake-up is lost. If it
    # ever is a descriptor that cannot be read (an epoll descriptor), the event loop's own
    # `wait_readable` does the waiting; that method is internal to the Crystal runtime and is used in
    # this one place only.
    private def self.watch(io : IO::FileDescriptor) : Nil
      buffer = Bytes.new(64)
      readable = true
      until io.closed?
        begin
          if readable
            break if io.read(buffer) == 0
          else
            Crystal::EventLoop.current.wait_readable(io)
          end
          wake
        rescue ex : IO::Error
          break if io.closed?
          # EINVAL: not a readable descriptor. Wait for readiness instead.
          raise ex unless readable
          readable = false
        end
      end
    end

    # Work posted from other threads (`set_later`) runs on its own fiber in this context.
    private def self.pump_posts : Nil
      loop do
        work = UI::POSTS.receive
        break if work == STOP_PUMP
        Wolkit.batch { Wolkit.guard { work.call } }
        Frame.flush_all
      end
    end
  end
end
