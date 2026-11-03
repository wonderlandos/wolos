# SPDX-License-Identifier: BSD-2-Clause
# FFI::Window: one libwolkit window (wolkit.h "windows"). The patch goes in as the bytes of a reused
# buffer, and the headless/test verbs (inject, advance, dump, render) are here too.
require "./call"
require "./engine"
require "./style_table"

module Wolkit
  module FFI
    class Window < Wol::Handle(LibWolkit::Window)
      # Window flag: the person cannot resize it.
      FIXED_SIZE = LibWolkit::WOLKIT_WINDOW_FIXED_SIZE

      # What `dump` prints.
      SCENE  = LibWolkit::WOLKIT_DUMP_SCENE
      LAYOUT = LibWolkit::WOLKIT_DUMP_LAYOUT
      A11Y   = LibWolkit::WOLKIT_DUMP_A11Y

      def self.create(engine : Engine, title : ::String, width : UInt32, height : UInt32, flags : UInt32 = 0_u32) : Window
        ptr = FFI.out(LibWolkit::Window) do |slot, err|
          LibWolkit.wolkit_window_create(engine, title.to_unsafe, title.bytesize, width, height, flags, slot, err)
        end
        new(ptr)
      end

      protected def release(ptr : Pointer(LibWolkit::Window)) : Nil
        LibWolkit.wolkit_window_release(ptr)
      end

      # The engine's id for this window (events carry it).
      def id : UInt32
        LibWolkit.wolkit_window_get_id(self)
      end

      def title=(title : ::String) : ::String
        FFI.call { |err| LibWolkit.wolkit_window_set_title(self, title.to_unsafe, title.bytesize, err) }
        title
      end

      def size_limits(min_w : UInt32, min_h : UInt32, max_w : UInt32, max_h : UInt32) : Nil
        FFI.call { |err| LibWolkit.wolkit_window_set_size_limits(self, min_w, min_h, max_w, max_h, err) }
      end

      def show : Nil
        FFI.call { |err| LibWolkit.wolkit_window_show(self, err) }
      end

      def close_window : Nil
        FFI.call { |err| LibWolkit.wolkit_window_close(self, err) }
      end

      # Apply one patch (atomic). The bytes are borrowed for the call.
      def apply(patch : Bytes) : Nil
        FFI.call { |err| LibWolkit.wolkit_window_apply(self, patch.to_unsafe, patch.size, err) }
      end

      def focus(node : UInt32) : Nil
        FFI.call { |err| LibWolkit.wolkit_window_set_focus(self, node, err) }
      end

      # Width, height in logical pixels and the scale factor.
      def size : {UInt32, UInt32, Float32}
        w = 0_u32
        h = 0_u32
        scale = 1.0_f32
        LibWolkit.wolkit_window_get_size(self, pointerof(w), pointerof(h), pointerof(scale))
        {w, h, scale}
      end

      # The text a part holds (a text field's content).
      def read_text(node : UInt32) : ::String
        read_sized { |buf, needed, err| LibWolkit.wolkit_window_read_text(self, node, buf.to_unsafe, buf.size, needed, err) }
      end

      # A text dump of the scene, the layout or the accessibility tree.
      def dump(what : Int32) : ::String
        read_sized { |buf, needed, err| LibWolkit.wolkit_window_dump(self, what, buf.to_unsafe, buf.size, needed, err) }
      end

      # Headless only: the pixels (RGBA) and their size.
      def render_rgba : {Bytes, UInt32, UInt32}
        w = 0_u32
        h = 0_u32
        size_w, size_h, scale = size
        pixels = Bytes.new((size_w * scale).ceil.to_i * (size_h * scale).ceil.to_i * 4)
        FFI.call { |err| LibWolkit.wolkit_window_render_rgba(self, pixels.to_unsafe, pixels.size, pointerof(w), pointerof(h), err) }
        {pixels, w, h}
      end

      # Headless only: a pointer event (kind WOLKIT_POINTER_*).
      def inject_pointer(kind : UInt32, x : Float32, y : Float32, button : UInt32 = 0_u32, mods : UInt32 = 0_u32) : Nil
        FFI.call { |err| LibWolkit.wolkit_window_inject_pointer(self, kind, x, y, button, mods, err) }
      end

      def inject_key(keysym : UInt32, pressed : Bool, mods : UInt32 = 0_u32) : Nil
        FFI.call { |err| LibWolkit.wolkit_window_inject_key(self, keysym, pressed ? 1_u32 : 0_u32, mods, err) }
      end

      def inject_text(text : ::String) : Nil
        FFI.call { |err| LibWolkit.wolkit_window_inject_text(self, text.to_unsafe, text.bytesize, err) }
      end

      # Headless only: move the engine clock forward by `ns` nanoseconds.
      def advance_time(ns : UInt64) : Nil
        FFI.call { |err| LibWolkit.wolkit_window_advance_time(self, ns, err) }
      end

      def style_table=(table : StyleTable) : StyleTable
        FFI.call { |err| LibWolkit.wolkit_window_set_style_table(self, table, err) }
        table
      end

      # The rectangle of a part: x, y, width, height in window pixels.
      def node_rect(node : UInt32) : {Float32, Float32, Float32, Float32}
        xywh = StaticArray(Float32, 4).new(0.0_f32)
        FFI.call { |err| LibWolkit.wolkit_window_node_rect(self, node, xywh.to_unsafe, err) }
        {xywh[0], xywh[1], xywh[2], xywh[3]}
      end

      # The part under a point, or 0 (the window itself).
      def hit_test(x : Float32, y : Float32) : UInt32
        node = 0_u32
        LibWolkit.wolkit_window_hit_test(self, x, y, pointerof(node))
        node
      end

      # A "needed" call: try 8 KiB, then once more with the size the engine asked for.
      private def read_sized(& : Bytes, LibC::SizeT*, LibWolkit::Error** -> Int32) : ::String
        needed = LibC::SizeT.new(0)
        buf = Bytes.new(8192)
        err = Pointer(LibWolkit::Error).null
        status = yield buf, pointerof(needed), pointerof(err)
        if status == LibWolkit::WOLKIT_ERR_LIMIT && needed > buf.size
          if !err.null?
            LibWolkit.wolkit_error_release(err)
            err = Pointer(LibWolkit::Error).null
          end
          buf = Bytes.new(needed)
          status = yield buf, pointerof(needed), pointerof(err)
        end
        FFI.check(status, err)
        ::String.new(buf[0, needed])
      end
    end
  end
end
