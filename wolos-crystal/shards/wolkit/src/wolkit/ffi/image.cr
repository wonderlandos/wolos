# SPDX-License-Identifier: BSD-2-Clause
# FFI::Image: a picture the engine holds (`wolkit_image_*`, 1.0 and 1.1). PNG and JPEG are decoded
# on the engine's worker threads with the EXIF orientation applied; until then `state` is
# `Pending` and an `image` part showing it draws a placeholder. `image picture: pic, alt: "…"`
# shows one; keep the object while a part shows it (the engine forgets a released picture).
# A picture named by path needs no object at all: `image src: "/path/to/a.jpg", alt: "…"`.
require "./call"
require "./engine"

module Wolkit
  module FFI
    class Image < Wol::Handle(LibWolkit::Image)
      enum State : UInt32
        Pending = 0
        Ready   = 1
        Failed  = 2
      end

      # A picture read from `path` (absolute) on a worker. `max_side` caps its longer side (0: none),
      # so a thumbnail decodes small. A missing or broken file shows as `Failed`, not an exception.
      def self.from_file(engine : Engine, path : ::String, max_side : Int = 0) : Image
        ptr = FFI.out(LibWolkit::Image) do |slot, err|
          LibWolkit.wolkit_image_create_file(engine, path.to_unsafe, path.bytesize, max_side.to_u32, slot, err)
        end
        new(ptr)
      end

      # A picture from PNG or JPEG bytes; the header is checked now, the pixels decode on a worker.
      def self.from_bytes(engine : Engine, bytes : Bytes, max_side : Int = 0) : Image
        ptr = FFI.out(LibWolkit::Image) do |slot, err|
          if max_side == 0
            LibWolkit.wolkit_image_create(engine, bytes.to_unsafe, bytes.size, slot, err)
          else
            LibWolkit.wolkit_image_create_encoded(engine, bytes.to_unsafe, bytes.size, max_side.to_u32, slot, err)
          end
        end
        new(ptr)
      end

      # A picture from straight RGBA pixels, `width * 4` bytes a row; ready at once.
      def self.from_rgba(engine : Engine, width : Int, height : Int, pixels : Bytes) : Image
        ptr = FFI.out(LibWolkit::Image) do |slot, err|
          LibWolkit.wolkit_image_create_rgba(engine, width.to_u32, height.to_u32, (width * 4).to_u32, pixels.to_unsafe, pixels.size, slot, err)
        end
        new(ptr)
      end

      # The id a part names it by (`WOLKIT_PROP_IMAGE_ID`).
      def id : UInt32
        LibWolkit.wolkit_image_get_id(self)
      end

      # Width and height, upright; nil until a file's header has been read.
      def size : {UInt32, UInt32}?
        w = 0_u32
        h = 0_u32
        {w, h} if LibWolkit.wolkit_image_get_size(self, pointerof(w), pointerof(h)) == LibWolkit::WOLKIT_OK
      end

      def state : State
        State.new(LibWolkit.wolkit_image_get_state(self))
      end

      protected def release(ptr : Pointer(LibWolkit::Image)) : Nil
        LibWolkit.wolkit_image_release(ptr)
      end
    end
  end

  # A picture from a file for `image picture:`, decoded off the UI thread (the engine backend only).
  def self.load_image(path : String, max_side : Int = 0) : FFI::Image
    FFI::Image.from_file(image_engine, path, max_side)
  end

  # A picture from PNG or JPEG bytes for `image picture:` (the engine backend only).
  def self.load_image(bytes : Bytes, max_side : Int = 0) : FFI::Image
    FFI::Image.from_bytes(image_engine, bytes, max_side)
  end

  private def self.image_engine : FFI::Engine
    backend = Runtime.backend
    raise UsageError.new("Pictures need the engine; this backend does not hold them.") unless backend.is_a?(EngineBackend)
    backend.engine
  end
end
