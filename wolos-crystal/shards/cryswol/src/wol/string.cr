# SPDX-License-Identifier: BSD-2-Clause
# Strings across the boundary (plan-cr1 §3 rules 5 and 6). In: pass the pointer and byte size; libwol
# copies, and NULL with length 0 is the empty string. Out: copy the bytes into a Crystal String at
# once and release the libwol string when the call handed over a reference. A libwol pointer is
# never kept. Named `Strings` because a `Wol::String` would hide `::String` inside `module Wol`.
require "./lib_wol"

module Wol
  module Strings
    # Copy a borrowed `wol_string` (a `*_get_*` result). NULL gives nil.
    def self.borrow(s : LibWol::String*) : ::String?
      return nil if s.null?
      len = LibC::SizeT.new(0)
      bytes = LibWol.wol_string_get_bytes(s, pointerof(len))
      ::String.new(bytes, len)
    end

    # Copy an owned `wol_string` (a `*_copy_*` or out-parameter result) and release it.
    # NULL gives nil.
    def self.take(s : LibWol::String*) : ::String?
      return nil if s.null?
      begin
        borrow(s)
      ensure
        LibWol.wol_string_release(s)
      end
    end

    # The pointer to pass for `s`: NULL for nil or the empty string (wol.h: NULL with length 0).
    def self.ptr(s : ::String?) : UInt8*
      s.nil? || s.empty? ? Pointer(UInt8).null : s.to_unsafe
    end

    # The byte length to pass for `s` (0 for nil).
    def self.size(s : ::String?) : LibC::SizeT
      LibC::SizeT.new(s.try(&.bytesize) || 0)
    end

    # Round-trip `s` through `wol_string_create`, for specs and for callers that must hand libwol a
    # string object. Raises `InvalidArgument` for text libwol refuses (for example invalid UTF-8).
    def self.round_trip(s : ::String) : ::String
      made = Wol.out(LibWol::String) { |slot, err| LibWol.wol_string_create(ptr(s), size(s), slot, err) }
      take(made) || ""
    end
  end
end
