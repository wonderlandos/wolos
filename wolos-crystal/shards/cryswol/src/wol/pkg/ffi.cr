# SPDX-License-Identifier: GPL-2.0-or-later
# The raw side of Wol::Pkg: every pointer, out-parameter and boxed block of the libwopkg calls lives
# here (RCR-010, FFI_ALLOW), so pkg.cr reads as plain Crystal. Same rules as libwol (wopkg.h header):
# an error object is read once and released, a result string is copied and released, and a progress
# block runs only during its call, so its box lives in that call's frame.
require "./lib_wopkg"
require "../error"
require "../callbacks"

module Wol
  module Pkg
    # A download or package whose signature or checksum does not hold (`WOPKG_ERR_UNTRUSTED`).
    class Untrusted < Wol::Error
    end

    # No package, record or history entry by that name (`WOPKG_ERR_NOT_FOUND`).
    class NotFound < Wol::Error
    end

    # The error class for a libwopkg `code`: the two of its own, else the `Wol::Error` mapping
    # (the numbers 1 to 4 are the same in wol.h and wopkg.h).
    def self.error(code : Int32, message : ::String) : Wol::Error
      case code
      when LibWopkg::WOPKG_ERR_UNTRUSTED then Untrusted.new(code, message)
      when LibWopkg::WOPKG_ERR_NOT_FOUND then NotFound.new(code, message)
      else                                    Wol::Error.from(code, message)
      end
    end

    module FFI
      # The progress trampoline: hands each event (JSON text, valid during the call) to the block
      # boxed in `user_data`. Exceptions go to `Wol::Loop.on_error`; nothing unwinds into C.
      PROGRESS = ->(json : UInt8*, len : LibC::SizeT, data : Void*) do
        Callbacks.guard do
          block = Box(Proc(::String, Nil)).unbox(data)
          block.call(::String.new(json, len)) unless json.null?
        end
        nil
      end

      # Raise for a failed `status`, with libwopkg's sentence, releasing `err`.
      def self.check(status : Int32, err : LibWopkg::Error*) : Nil
        return if status == LibWopkg::WOPKG_OK
        raise Internal.new(status, FALLBACK_MESSAGE) if err.null?
        code = LibWopkg.wopkg_error_get_code(err)
        len = LibC::SizeT.new(0)
        bytes = LibWopkg.wopkg_error_get_message(err, pointerof(len))
        message = bytes.null? ? FALLBACK_MESSAGE : ::String.new(bytes, len)
        LibWopkg.wopkg_error_release(err)
        raise Pkg.error(code, message)
      end

      # Copy an owned result string and release it.
      def self.take(s : LibWopkg::String*) : ::String
        return "" if s.null?
        begin
          len = LibC::SizeT.new(0)
          bytes = LibWopkg.wopkg_string_get_bytes(s, pointerof(len))
          bytes.null? ? "" : ::String.new(bytes, len)
        ensure
          LibWopkg.wopkg_string_release(s)
        end
      end

      # One call with a result string: the block gets the `wopkg_string**` and `wopkg_error**`
      # to pass and returns the status. Returns the result text or raises.
      def self.result(& : LibWopkg::String**, LibWopkg::Error** -> Int32) : ::String
        slot = Pointer(LibWopkg::String).null
        err = Pointer(LibWopkg::Error).null
        status = yield pointerof(slot), pointerof(err)
        check(status, err)
        take(slot)
      end

      # One call with no result: the block gets the `wopkg_error**` and returns the status.
      def self.call(& : LibWopkg::Error** -> Int32) : Nil
        err = Pointer(LibWopkg::Error).null
        status = yield pointerof(err)
        check(status, err)
      end

      # A new context for `config` (JSON). Raises `InvalidArgument` for a bad config.
      def self.context(config : ::String) : LibWopkg::Context*
        ptr = Pointer(LibWopkg::Context).null
        err = Pointer(LibWopkg::Error).null
        check(LibWopkg.wopkg_context_create(Strings.ptr(config), Strings.size(config), pointerof(ptr), pointerof(err)), err)
        ptr
      end

      # Run a call that reports progress: the block gets the trampoline and the `user_data` that
      # carries `progress`. The box stays referenced by this frame for as long as libwopkg may
      # call it (only during the call, wopkg.h).
      def self.with_progress(progress : ::String ->, & : LibWopkg::ProgressFn, Void* -> T) : T forall T
        box = Box(Proc(::String, Nil)).box(progress)
        yield PROGRESS, box
      end
    end
  end
end
