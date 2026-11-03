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

    # Run until the last window closes or the engine says quit.
    def self.run(engine : FFI::Engine) : Nil
      UI.claim
      Frame.auto = true
      wake = engine.wake_io
      pump = spawn(name: "wolkit-posts") { pump_posts }
      GCPolicy.start
      begin
        until Runtime.quit?
          turn(engine)
          break if Runtime.quit?
          wait_readable(wake)
        end
      ensure
        Frame.auto = false
        GCPolicy.stop
        wake.close
        pump.dead? || Wolkit::UI::POSTS.close
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

    # Block this fiber until `io` is readable. Crystal has no public API for this on a descriptor
    # that cannot be read (an epoll fd); the event loop's own method does it. Guarded by
    # spec/engine/loop_spec.cr so a Crystal upgrade that moves it fails a test, not an app.
    private def self.wait_readable(io : IO::FileDescriptor) : Nil
      Crystal::EventLoop.current.wait_readable(io)
    end

    # Work posted from other threads (`set_later`) runs on its own fiber in this context.
    private def self.pump_posts : Nil
      loop do
        work = UI::POSTS.receive? || break
        Wolkit.batch { Wolkit.guard { work.call } }
        Frame.flush_all
      end
    end
  end
end
