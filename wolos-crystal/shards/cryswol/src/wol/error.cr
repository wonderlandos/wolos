# SPDX-License-Identifier: BSD-2-Clause
# Errors: every failing libwol call becomes a `Wol::Error` subclass that carries libwol's own
# sentence (docs/modules/cryswol.md "Errors", plan-cr1 §4). The sentence is already written for a
# person, so the binding never adds text to it. `UsageError` is the one class whose text comes from
# Cryswol; it reports a programming mistake and is never shown to a person.
require "./lib_wol"

module Wol
  # A libwol call failed. `code` is the `WOL_ERR_*` status, `message` the sentence libwol wrote.
  class Error < Exception
    # The `WOL_ERR_*` status of the failed call.
    getter code : Int32

    def initialize(@code : Int32, message : ::String)
      super(message)
    end

    # The subclass for `code`: `InvalidArgument`, `PermissionDenied`, `Unavailable` or `Internal`
    # (any code this version does not know, too, so a newer libwol still maps to a `Wol::Error`).
    def self.from(code : Int32, message : ::String) : Error
      case code
      when LibWol::WOL_ERR_INVALID_ARGUMENT  then InvalidArgument.new(code, message)
      when LibWol::WOL_ERR_PERMISSION_DENIED then PermissionDenied.new(code, message)
      when LibWol::WOL_ERR_UNAVAILABLE       then Unavailable.new(code, message)
      else                                        Internal.new(code, message)
      end
    end
  end

  # An argument libwol refused (`WOL_ERR_INVALID_ARGUMENT`).
  class InvalidArgument < Error
  end

  # The person or the system said no (`WOL_ERR_PERMISSION_DENIED`).
  class PermissionDenied < Error
  end

  # The system service or the Kit host is not there (`WOL_ERR_UNAVAILABLE`).
  class Unavailable < Error
  end

  # Something inside libwol broke (`WOL_ERR_INTERNAL`, or a code this version does not know).
  class Internal < Error
  end

  # A programming mistake in the app: a closed handle, an object used after its callback, or a
  # second dispatch from inside a callback. Developer-facing text only.
  class UsageError < Exception
  end

  # Used when libwol failed without an error object (it always passes one; this is the guard).
  FALLBACK_MESSAGE = "Something went wrong inside the app library."

  # Raise for a failed `status`, taking the sentence from `err` and releasing it. Returns on
  # `WOL_OK`. Every verb goes through this (or `Wol.call`/`Wol.out`, which call it).
  def self.check(status : Int32, err : LibWol::Error*) : Nil
    return if status == LibWol::WOL_OK
    if err.null?
      raise Internal.new(status, FALLBACK_MESSAGE)
    end
    code = LibWol.wol_error_get_code(err)
    len = LibC::SizeT.new(0)
    bytes = LibWol.wol_error_get_message(err, pointerof(len))
    message = bytes.null? ? FALLBACK_MESSAGE : ::String.new(bytes, len)
    LibWol.wol_error_release(err)
    raise Error.from(code, message)
  end

  # Run one fallible libwol call: the block gets the `wol_error**` to pass last and returns the
  # status. Raises the mapped `Wol::Error` on failure.
  #
  # ```
  # Wol.call { |err| LibWol.wol_window_show(window, err) }
  # ```
  def self.call(& : LibWol::Error** -> Int32) : Nil
    err = Pointer(LibWol::Error).null
    status = yield pointerof(err)
    check(status, err)
  end

  # Run one fallible libwol call with an out-parameter of type `T*`: the block gets the `T**` and
  # the `wol_error**`. Returns the out value, which is NULL when the call means "none" (an unset
  # preference, a cancelled panel). Raises on failure.
  #
  # ```
  # ptr = Wol.out(LibWol::Window) { |slot, err| LibWol.wol_window_create(ctx, t, t.bytesize, slot, err) }
  # ```
  def self.out(type : T.class, & : Pointer(Pointer(T)), LibWol::Error** -> Int32) : Pointer(T) forall T
    ptr = Pointer(T).null
    err = Pointer(LibWol::Error).null
    status = yield pointerof(ptr), pointerof(err)
    check(status, err)
    ptr
  end

  # Run one fallible libwol call with two number out-parameters (`wol_color_pick`): the block gets
  # the two pointers and the `wol_error**`. Returns the two values. Raises on failure.
  def self.scalars(a : A.class, b : B.class, & : Pointer(A), Pointer(B), LibWol::Error** -> Int32) : {A, B} forall A, B
    x = A.zero
    y = B.zero
    err = Pointer(LibWol::Error).null
    status = yield pointerof(x), pointerof(y), pointerof(err)
    check(status, err)
    {x, y}
  end
end
