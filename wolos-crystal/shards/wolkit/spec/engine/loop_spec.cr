# SPDX-License-Identifier: BSD-2-Clause
# The real loop: `Wolkit::Loop.run` watches the engine's descriptor on its own fiber, takes a turn
# when woken (by the engine, `Loop.wake` or `Runtime.quit`), runs the handlers in one batch and sends
# one patch, and other fibers (a ticker here) keep running while it waits. A helper fiber plays the
# person: it clicks, then asks the loop to end.
require "./engine_helper_spec"
require "../dsl_app_spec"

describe Wolkit::Loop do
  it "takes a turn when woken, handles the click, and ends at once when asked to quit" do
    engine = Wolkit::FFI::Engine.create("org.wolos.spec", Wolkit::FFI::Engine::HEADLESS | Wolkit::FFI::Engine::FORCE_CPU)
    Wolkit::Runtime.backend = Wolkit::EngineBackend.new(engine)
    app = SpecTodos.new
    app.start
    Wolkit::Frame.flush_all
    window = Wolkit::Runtime.windows.first.backend.as(Wolkit::EngineWindow).handle
    ticks = 0
    spawn do
      sleep 30.milliseconds
      app.draft.value = "milk"
      # The fiber that sends the change runs while this one sleeps.
      sleep 20.milliseconds
      x, y, w, h = window.node_rect(window_node(window, "Add"))
      window.inject_pointer(LibWolkit::WOLKIT_POINTER_MOVE.to_u32, x + w / 2, y + h / 2)
      window.inject_pointer(LibWolkit::WOLKIT_POINTER_DOWN.to_u32, x + w / 2, y + h / 2, 1_u32)
      window.inject_pointer(LibWolkit::WOLKIT_POINTER_UP.to_u32, x + w / 2, y + h / 2, 1_u32)
      # The headless engine queues an injected event on the calling thread without writing to its
      # descriptor; the real backends do write. Wake the loop the way they would.
      Wolkit::Loop.wake
      sleep 30.milliseconds
      Wolkit::Runtime.quit
    end
    spawn do
      3.times do
        sleep 10.milliseconds
        ticks += 1
      end
    end
    started = Time.instant
    Wolkit::Loop.run(engine)
    (Time.instant - started).should be < 2.seconds
    app.items.to_a.should eq ["milk"]
    ticks.should eq 3
  ensure
    Wolkit::Runtime.reset
    Wolkit::Scope.reset_root
    Wolkit::Runtime.backend = nil
    engine.try &.close
  end

  it "runs an app with Wolkit.run until its last window is closed" do
    closed = false
    spawn do
      sleep 40.milliseconds
      window = Wolkit::Runtime.windows.first
      window.close
      closed = true
    end
    started = Time.instant
    Wolkit.run(SpecTodos, "org.wolos.spec", Wolkit::FFI::Engine::HEADLESS | Wolkit::FFI::Engine::FORCE_CPU)
    closed.should be_true
    (Time.instant - started).should be < 2.seconds
    Wolkit::Runtime.windows.should be_empty
  end
end

# The id of the part with this accessible name, from the engine's own accessibility dump.
def window_node(window : Wolkit::FFI::Window, name : String) : UInt32
  window.dump(Wolkit::FFI::Window::A11Y).each_line do |line|
    if match = line.match(/^\s*(\d+) \w+ role=\w+ name="#{Regex.escape(name)}"/)
      return match[1].to_u32
    end
  end
  raise "no part named #{name}"
end
