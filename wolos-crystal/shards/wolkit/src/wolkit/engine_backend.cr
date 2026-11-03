# SPDX-License-Identifier: BSD-2-Clause
# The real backend: windows and patches go to libwolkit. EngineBackend makes windows on an
# FFI::Engine; EngineWindow adapts an FFI::Window to the WindowBackend the runtime talks to.
require "./window"
require "./ffi/engine"
require "./ffi/window"
require "./style/kit_table"

module Wolkit
  class EngineWindow < WindowBackend
    getter handle : FFI::Window
    @id : UInt32

    def initialize(@handle : FFI::Window)
      @id = @handle.id
    end

    def id : UInt32
      @id
    end

    def apply(bytes : Bytes) : Nil
      @handle.apply(bytes)
    end

    def show : Nil
      @handle.show
    end

    def close : Nil
      @handle.close_window
      @handle.close
    end

    def title=(title : String) : String
      @handle.title = title
    end

    def size : {UInt32, UInt32, Float32}
      @handle.size
    end
  end

  class EngineBackend < Backend
    getter engine : FFI::Engine
    @table : FFI::StyleTable? = nil
    @windows = [] of EngineWindow

    # Every window starts with the default look (`KIT_TABLE`, compiled from styles/kit.wolcss).
    def initialize(@engine : FFI::Engine)
      @table = FFI::StyleTable.load(@engine, KIT_TABLE)
    end

    def open_window(title : String, width : Int32, height : Int32, limits : {Int32, Int32, Int32, Int32}?) : WindowBackend
      handle = FFI::Window.create(@engine, title, width.to_u32, height.to_u32)
      limits.try { |l| handle.size_limits(l[0].to_u32, l[1].to_u32, l[2].to_u32, l[3].to_u32) }
      @table.try { |table| handle.style_table = table }
      window = EngineWindow.new(handle)
      @windows << window
      window
    end

    # Use the compiled style table `bytes` (`build/styles.wsb`) for every window, open now or later. An
    # app that has its own sheets compiles the kit sheet first (`build/00-kit.wolcss`, see the template's
    # `make styles`), so the kit class ids stay where the DSL expects them; a table made without it
    # replaces the default look.
    def use_styles(bytes : Bytes) : Nil
      table = FFI::StyleTable.load(@engine, bytes)
      @table = table
      @windows.reject!(&.handle.closed?)
      @windows.each(&.handle.style_table=(table))
    end
  end

  # Use the app's compiled style sheet (the `build/styles.wsb` that `wolkit-stylec` writes). Call it
  # in `view` before the first `window`, or at any time to restyle every open window.
  def self.use_styles(bytes : Bytes) : Nil
    backend = Runtime.backend
    raise UsageError.new("Style sheets need the engine; this backend does not take them.") unless backend.is_a?(EngineBackend)
    backend.use_styles(bytes)
  end

  # Read a compiled style sheet from `path` and use it; see `use_styles`.
  def self.use_styles(path : String) : Nil
    use_styles(File.read(path).to_slice)
  end
end
