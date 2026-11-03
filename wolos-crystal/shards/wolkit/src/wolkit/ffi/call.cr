# SPDX-License-Identifier: BSD-2-Clause
# The FFI layer's call helpers: every failing libwolkit call becomes an exception that carries the
# engine's own sentence (wolkit.h: "one sentence for a person"), as Cryswol does for libwol. The
# engine adds two statuses libwol lacks, LIMIT (a documented limit) and DEVICE_LOST.
require "../lib_wolkit"
require "../../../../cryswol/src/cryswol"

module Wolkit
  # A documented limit of the engine was reached (a window part count, a patch size).
  class EngineLimit < Wol::Error
  end

  # The graphics device went away and could not be recovered.
  class DeviceLost < Wol::Error
  end

  module FFI
    # Turn a failed `status` into the matching exception, taking the sentence from `err` and
    # releasing it. Returns on WOLKIT_OK.
    def self.check(status : Int32, err : LibWolkit::Error*) : Nil
      return if status == LibWolkit::WOLKIT_OK
      if err.null?
        raise Wol::Internal.new(status, Wol::FALLBACK_MESSAGE)
      end
      code = LibWolkit.wolkit_error_get_code(err)
      len = LibC::SizeT.new(0)
      bytes = LibWolkit.wolkit_error_get_message(err, pointerof(len))
      message = bytes.null? ? Wol::FALLBACK_MESSAGE : ::String.new(bytes, len)
      LibWolkit.wolkit_error_release(err)
      raise error_for(code, message)
    end

    def self.error_for(code : Int32, message : ::String) : Wol::Error
      case code
      when LibWolkit::WOLKIT_ERR_LIMIT       then EngineLimit.new(code, message)
      when LibWolkit::WOLKIT_ERR_DEVICE_LOST then DeviceLost.new(code, message)
      else                                        Wol::Error.from(code, message)
      end
    end

    # Run one fallible call: the block gets the `wolkit_error**` to pass last and returns the status.
    def self.call(& : LibWolkit::Error** -> Int32) : Nil
      err = Pointer(LibWolkit::Error).null
      status = yield pointerof(err)
      check(status, err)
    end

    # Run one fallible call with an out-parameter of type `T*`.
    def self.out(type : T.class, & : Pointer(Pointer(T)), LibWolkit::Error** -> Int32) : Pointer(T) forall T
      ptr = Pointer(T).null
      err = Pointer(LibWolkit::Error).null
      status = yield pointerof(ptr), pointerof(err)
      check(status, err)
      ptr
    end
  end
end
