# SPDX-License-Identifier: BSD-2-Clause
# The real backend: windows and patches go to libwolkit. EngineBackend makes windows on an
# FFI::Engine; EngineWindow adapts an FFI::Window to the WindowBackend the runtime talks to.
require "./window"
require "./ffi/engine"
require "./ffi/window"

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

    def initialize(@engine : FFI::Engine)
    end

    def open_window(title : String, width : Int32, height : Int32, limits : {Int32, Int32, Int32, Int32}?) : WindowBackend
      handle = FFI::Window.create(@engine, title, width.to_u32, height.to_u32)
      limits.try { |l| handle.size_limits(l[0].to_u32, l[1].to_u32, l[2].to_u32, l[3].to_u32) }
      EngineWindow.new(handle)
    end
  end
end
