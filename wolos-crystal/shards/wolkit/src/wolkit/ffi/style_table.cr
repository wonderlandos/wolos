# SPDX-License-Identifier: BSD-2-Clause
# FFI::StyleTable: a loaded style table (WSB1 bytes, from `wolkit-stylec` or `wolkit_style_compile`).
# Hot reload loads a new one and sets it on the window; no restart.
require "./call"
require "./engine"

module Wolkit
  module FFI
    class StyleTable < Wol::Handle(LibWolkit::StyleTable)
      # Load compiled style table bytes into `engine`. Raises with the engine's sentence on bad bytes.
      def self.load(engine : Engine, bytes : Bytes) : StyleTable
        ptr = FFI.out(LibWolkit::StyleTable) do |slot, err|
          LibWolkit.wolkit_style_table_load(engine, bytes.to_unsafe, bytes.size, slot, err)
        end
        new(ptr)
      end

      protected def release(ptr : Pointer(LibWolkit::StyleTable)) : Nil
        LibWolkit.wolkit_style_table_release(ptr)
      end

      # Compile `.wolcss` source to table bytes (the dev loop and `make styles`).
      def self.compile(source : ::String) : Bytes
        needed = LibC::SizeT.new(0)
        buf = Bytes.new(64 * 1024)
        err = Pointer(LibWolkit::Error).null
        status = LibWolkit.wolkit_style_compile(source.to_unsafe, source.bytesize, buf.to_unsafe, buf.size, pointerof(needed), pointerof(err))
        if status == LibWolkit::WOLKIT_ERR_LIMIT && needed > buf.size
          LibWolkit.wolkit_error_release(err) unless err.null?
          err = Pointer(LibWolkit::Error).null
          buf = Bytes.new(needed)
          status = LibWolkit.wolkit_style_compile(source.to_unsafe, source.bytesize, buf.to_unsafe, buf.size, pointerof(needed), pointerof(err))
        end
        FFI.check(status, err)
        buf[0, needed].dup
      end
    end
  end
end
