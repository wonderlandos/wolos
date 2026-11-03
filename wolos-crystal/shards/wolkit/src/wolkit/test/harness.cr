# SPDX-License-Identifier: BSD-2-Clause
# `require "wolkit/test"`: a headless session for specs (plan-wolkit D4). It starts the real engine
# without a display, mounts an app, and gives `click`, `type`, `text`, `a11y_snapshot`, `advance` and
# `screenshot`. There is no fake wold and no Kit host: the engine lays out, hit-tests and renders on
# the CPU, bit-exactly, so a screenshot is the same on every machine.
#
# ```
# Wolkit::Test.run(Todos) do |t|
#   t.type "milk", into: "What needs doing?"
#   t.click "Add"
#   t.text("1 left").should eq "1 left"
# end
# ```
require "../runtime"
require "../loop"
require "./recorder"
require "./png"

module Wolkit
  module Test
    # One headless engine, one mounted app, and the helpers that act on it.
    class Session
      getter engine : FFI::Engine
      getter app : App

      def initialize(@app : App, flags : UInt32 = 0_u32)
        UI.claim
        @engine = FFI::Engine.create("org.wolos.spec", FFI::Engine::HEADLESS | FFI::Engine::FORCE_CPU | flags)
        Runtime.backend = EngineBackend.new(@engine)
        @app.start
        Frame.flush_all
        settle
      end

      # The first open window.
      def window : FFI::Window
        Runtime.windows.first.backend.as(EngineWindow).handle
      end

      # Let the engine run what it has, hand its events to the app, send the app's changes.
      def settle : Nil
        Loop.turn(@engine)
      end

      # Click the part whose accessible name is `name`: a pointer press and release at its centre.
      def click(name : String) : Nil
        x, y = center(name)
        window.inject_pointer(LibWolkit::WOLKIT_POINTER_MOVE.to_u32, x, y)
        window.inject_pointer(LibWolkit::WOLKIT_POINTER_DOWN.to_u32, x, y, 1_u32)
        window.inject_pointer(LibWolkit::WOLKIT_POINTER_UP.to_u32, x, y, 1_u32)
        settle
      end

      # Type `text` (into the part named `into` when given: it is clicked first).
      def type(text : String, into : String? = nil) : Nil
        click(into) if into
        window.inject_text(text)
        settle
      end

      # Press a key (an X11 keysym) with modifiers.
      def press(keysym : UInt32, mods : UInt32 = 0_u32) : Nil
        window.inject_key(keysym, true, mods)
        window.inject_key(keysym, false, mods)
        settle
      end

      # The text a part shows, found by accessible name or by its own text.
      def text(name : String) : String
        node = find(name)
        window.read_text(node)
      end

      # The accessibility tree as text.
      def a11y_snapshot : String
        window.dump(FFI::Window::A11Y)
      end

      # The scene as text.
      def scene : String
        window.dump(FFI::Window::SCENE)
      end

      # Move the engine clock forward and let the app react (animations, timers in the engine).
      def advance(span : Time::Span) : Nil
        window.advance_time(span.total_nanoseconds.to_u64)
        settle
      end

      # The window as a PNG (CPU-rasterised, the same bytes on every machine).
      def screenshot : Bytes
        pixels, w, h = window.render_rgba
        PNG.encode(pixels, w.to_i, h.to_i)
      end

      # The part named `name` (accessible name or text).
      def find(name : String) : UInt32
        a11y_snapshot.each_line do |line|
          if m = line.match(/^\s*(\d+) \w+ role=\w+ name="((?:[^"\\]|\\.)*)"/)
            return m[1].to_u32 if m[2] == name
          end
        end
        scene.each_line do |line|
          if m = line.match(/^\s*(\d+) \w+ .*"((?:[^"\\]|\\.)*)"\s*$/)
            return m[1].to_u32 if m[2] == name
          end
        end
        raise ArgumentError.new("No part is named #{name.inspect}. The accessibility tree is:\n#{a11y_snapshot}")
      end

      private def center(name : String) : {Float32, Float32}
        x, y, w, h = window.node_rect(find(name))
        {x + w / 2, y + h / 2}
      end

      def close : Nil
        Runtime.reset
        Scope.reset_root
        Runtime.backend = nil
        @engine.close
      end
    end

    # Mount `app_class` in a headless engine, yield the session, close it.
    def self.run(app_class : T.class, & : Session ->) : Nil forall T
      session = Session.new(app_class.new)
      begin
        yield session
      ensure
        session.close
      end
    end
  end
end
