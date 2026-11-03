# SPDX-License-Identifier: BSD-2-Clause
# Window and Runtime: a window is a mount plus the engine's window; the runtime knows the backend (the
# real engine, or a recorder in specs), every open window by id, and when to stop. The `window` macro
# of the DSL calls `Window.open`, which builds the window's content inside the block.
require "./mount"
require "./reactive/scope"
require "./event"
require "./env"

module Wolkit
  # The engine side of one window (or a stand-in for it in specs).
  abstract class WindowBackend
    # The engine's id for the window (events carry it).
    abstract def id : UInt32

    # Send one patch.
    abstract def apply(bytes : Bytes) : Nil

    abstract def show : Nil

    abstract def close : Nil

    abstract def title=(title : String) : String

    # The size in logical pixels and the scale factor.
    abstract def size : {UInt32, UInt32, Float32}
  end

  # The engine (or a stand-in) that makes windows.
  abstract class Backend
    abstract def open_window(title : String, width : Int32, height : Int32, limits : {Int32, Int32, Int32, Int32}?) : WindowBackend
  end

  class Window
    getter backend : WindowBackend
    getter mount : Mount
    getter scope : Scope
    getter title : String
    getter? closed : Bool = false
    @on_close : Proc(Nil)?

    def initialize(@backend : WindowBackend, @title : String, @on_close : Proc(Nil)? = nil)
      @scope = Scope.new
      @mount = Mount.new(->(bytes : Bytes) { @backend.apply(bytes) })
    end

    # Open a window and build its content with the block (the `window` macro calls this). The
    # content is mounted in one batch and sent as one patch before the window is shown.
    def self.open(title : String, width : Int32 = 640, height : Int32 = 480, min_width : Int32? = nil,
                  min_height : Int32? = nil, max_width : Int32? = nil, max_height : Int32? = nil,
                  on_close : Proc(Nil)? = nil, & : ->) : Window
      limits = (min_width || min_height || max_width || max_height) ? {min_width || 0, min_height || 0, max_width || 0, max_height || 0} : nil
      backend = Runtime.backend.open_window(title, width, height, limits)
      window = new(backend, title, on_close)
      Runtime.register(window)
      window.mount_content { yield }
      window.mount.flush
      backend.show
      window
    end

    # Run the block with this window's mount and scope current, in a batch.
    def mount_content(& : ->) : Nil
      Wolkit.batch do
        @scope.run do
          Mount.with(@mount) { yield }
        end
      end
    end

    # Give an event of this window to its mount; window-level events are handled here.
    def handle(event : Event) : Nil
      case event.kind
      when Wire::Event::CLOSE_REQUEST
        if handler = @on_close
          Wolkit.batch { Wolkit.guard { handler.call } }
        else
          close
        end
      when Wire::Event::THEME_CHANGED
        Env.apply_theme(event.u32(0) != 0, event.u32(4), event.u32(8))
      when Wire::Event::RESIZE
        _w, _h, scale = event.size
        Env.apply_scale(scale)
      when Wire::Event::DIAGNOSTIC
        Wolkit.diagnostic(event.u32(0), event.message)
      else
        @mount.dispatch(event)
      end
    end

    # Close the window and dispose everything in it.
    def close : Nil
      return if @closed
      @closed = true
      @scope.dispose
      @backend.close
      Runtime.unregister(self)
    end
  end

  module Runtime
    @@backend : Backend? = nil
    @@windows = {} of UInt32 => Window
    @@quit = false
    @@on_quit : Proc(Nil)? = nil

    def self.backend : Backend
      @@backend || raise UsageError.new("Wolkit is not running. Start the app with `Wolkit.run MyApp`, or use `Wolkit::Test.run` in a spec.")
    end

    def self.backend=(backend : Backend?) : Backend?
      @@backend = backend
    end

    def self.register(window : Window) : Nil
      @@windows[window.backend.id] = window
    end

    def self.unregister(window : Window) : Nil
      @@windows.delete(window.backend.id)
      quit if @@windows.empty?
    end

    def self.window(id : UInt32) : Window?
      @@windows[id]?
    end

    def self.windows : Array(Window)
      @@windows.values
    end

    # Ask the loop to end (and wake it, so it notices at once).
    def self.quit : Nil
      @@quit = true
      @@on_quit.try &.call
    end

    # What `quit` calls to wake the loop (set by `Wolkit::Loop.run`).
    def self.on_quit=(hook : Proc(Nil)?) : Proc(Nil)?
      @@on_quit = hook
    end

    def self.quit? : Bool
      @@quit
    end

    # Forget everything (specs and the end of a run).
    def self.reset : Nil
      @@windows.each_value(&.scope.dispose)
      @@windows.clear
      @@quit = false
    end

    # Give a decoded event to the window it is for.
    def self.dispatch(event : Event) : Nil
      @@windows[event.window]?.try &.handle(event)
    end
  end
end
