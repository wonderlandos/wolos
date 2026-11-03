# SPDX-License-Identifier: BSD-2-Clause
# FFI::Engine: the libwolkit engine handle (wolkit.h "engine"). It creates the engine, exposes the
# descriptor the loop waits on, dispatches, and drains the event stream into a reused buffer. All
# pointer handling lives here and in the sibling ffi files (RCR-010, FFI_ALLOW).
require "./call"

module Wolkit
  module FFI
    class Engine < Wol::Handle(LibWolkit::Engine)
      # Engine flags (WOLKIT_ENGINE_*).
      HEADLESS  = LibWolkit::WOLKIT_ENGINE_HEADLESS
      FORCE_CPU = LibWolkit::WOLKIT_ENGINE_FORCE_CPU
      FORCE_KIT = LibWolkit::WOLKIT_ENGINE_FORCE_KIT

      # Bytes handed to `wolkit_engine_read_events`: one record is at most 65,792 bytes, so this
      # always makes progress.
      EVENT_BUFFER = 256 * 1024

      @events = Bytes.new(EVENT_BUFFER)

      # Create an engine for `app_id` (reverse-DNS). Raises on a bad id or an engine that cannot start.
      def self.create(app_id : ::String, flags : UInt32 = 0_u32) : Engine
        ptr = FFI.out(LibWolkit::Engine) do |slot, err|
          LibWolkit.wolkit_engine_create(app_id.to_unsafe, app_id.bytesize, flags, slot, err)
        end
        new(ptr)
      end

      protected def release(ptr : Pointer(LibWolkit::Engine)) : Nil
        LibWolkit.wolkit_engine_release(ptr)
      end

      # A duplicate of the engine's wake descriptor that Crystal owns (closing it never touches the
      # engine's own descriptor; same reason as `Wol.dup_fd`).
      def wake_io : IO::FileDescriptor
        Wol.dup_fd(LibWolkit.wolkit_engine_get_fd(self))
      end

      # Run what the engine has queued, waiting up to `timeout_ms` (0 not at all, -1 forever).
      # Returns the WOLKIT_DISPATCH_* flags.
      def dispatch(timeout_ms : Int32 = 0) : UInt32
        flags = 0_u32
        FFI.call { |err| LibWolkit.wolkit_engine_dispatch(self, timeout_ms, pointerof(flags), err) }
        flags
      end

      # Move pending event records into the reused buffer; returns a reader over them. Whole
      # records only, so a call that fills the buffer is followed by another.
      def read_events : EventReader
        length = LibWolkit.wolkit_engine_read_events(self, @events.to_unsafe, @events.size)
        EventReader.new(@events, length.to_i)
      end

      # One engine fact (WOLKIT_INFO_*): the backend, the patch format, the limits.
      def info(key : Int32) : UInt64
        LibWolkit.wolkit_engine_get_info(self, key)
      end

      # Memory Saver: drop caches (WOLKIT_TRIM_*).
      def trim(level : UInt32) : Nil
        LibWolkit.wolkit_engine_trim(self, level)
      end

      # Headless only: set the environment the engine reports (dark, accent colour, flags).
      def set_environment(dark : Bool, accent_rgb : UInt32, flags : UInt32 = 0_u32) : Nil
        FFI.call { |err| LibWolkit.wolkit_engine_set_environment(self, dark ? 1_u32 : 0_u32, accent_rgb, flags, err) }
      end

      # The engine's frame statistics as JSON.
      def stats_json : ::String
        needed = LibC::SizeT.new(0)
        buf = Bytes.new(4096)
        status = LibWolkit.wolkit_engine_stats_json(self, buf.to_unsafe, buf.size, pointerof(needed), nil)
        if status != LibWolkit::WOLKIT_OK
          buf = Bytes.new(needed)
          FFI.call { |err| LibWolkit.wolkit_engine_stats_json(self, buf.to_unsafe, buf.size, pointerof(needed), err) }
        end
        ::String.new(buf[0, needed])
      end
    end
  end
end
