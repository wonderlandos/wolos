# SPDX-License-Identifier: BSD-2-Clause
# Info: a service's answer, one read-only tree (wol.h "1.4: answers"). The root is owned and
# released once; every child is borrowed from the root and holds a reference to it, so a child can
# never outlive the tree it points into (plan-cr1 §3 rule 2). Unknown keys are normal (answers grow
# in later versions), so lookups give nil rather than raising.
require "json"
require "./lib_wol"
require "./error"
require "./string"

module Wol
  class Info
    include Enumerable(Info)

    @ptr : LibWol::Info*
    # The owned root; nil when this is the root.
    @root : Info?

    # Wrap an owned answer (an out-parameter result). NULL gives nil.
    def self.take(ptr : LibWol::Info*) : Info?
      ptr.null? ? nil : new(ptr, nil)
    end

    # Run a call that answers an owned `wol_info` (the block gets the out pointer and the error
    # pointer, and returns the status) and wrap the answer. Raises the mapped `Wol::Error` on
    # failure, and `Internal` when libwol reports success with no answer.
    #
    # ```
    # Info.copy { |slot, err| LibWol.wol_power_copy_info(ctx, slot, err) }
    # ```
    def self.copy(& : LibWol::Info**, LibWol::Error** -> Int32) : Info
      ptr = Wol.out(LibWol::Info) { |slot, err| yield slot, err }
      take(ptr) || raise Internal.new(LibWol::WOL_ERR_INTERNAL, FALLBACK_MESSAGE)
    end

    protected def initialize(@ptr : LibWol::Info*, @root : Info?)
    end

    # True after `close` (of this tree's root).
    def closed? : Bool
      (@root || self).@ptr.null?
    end

    # Release the tree now. Children made from it raise `UsageError` afterwards. Closing a child
    # does nothing.
    def close : Nil
      return if @root || @ptr.null?
      ptr = @ptr
      @ptr = Pointer(LibWol::Info).null
      LibWol.wol_info_release(ptr)
    end

    def finalize
      LibWol.wol_info_release(@ptr) if @root.nil? && !@ptr.null?
    end

    # Field `key` of an object; nil when there is none (or this is not an object).
    def []?(key : ::String) : Info?
      child(LibWol.wol_info_get(live, Strings.ptr(key), Strings.size(key)))
    end

    # Field `key`; raises `KeyError` when there is none.
    def [](key : ::String) : Info
      self[key]? || raise KeyError.new("no #{key.inspect} in this answer")
    end

    # Item `index` of a list; nil when out of range (or this is not a list).
    def []?(index : Int) : Info?
      return nil if index < 0
      child(LibWol.wol_info_get_item(live, LibC::SizeT.new(index)))
    end

    # Item `index`; raises `IndexError` when out of range.
    def [](index : Int) : Info
      self[index]? || raise IndexError.new
    end

    # The number of items of a list; 0 for anything else.
    def size : Int32
      LibWol.wol_info_get_count(live).to_i32
    end

    # Each item of a list (nothing for anything else).
    def each(& : Info ->) : Nil
      size.times { |i| self[i]?.try { |item| yield item } }
    end

    # The text of a string; nil for anything else.
    def as_s? : ::String?
      Strings.borrow(LibWol.wol_info_get_text(live))
    end

    # The text; raises `TypeCastError` for anything but a string.
    def as_s : ::String
      as_s? || raise TypeCastError.new("this answer value is not text")
    end

    # A number; nil for anything else.
    def as_f? : Float64?
      value = 0.0
      LibWol.wol_info_get_number(live, pointerof(value)) == 1 ? value : nil
    end

    # The number; raises `TypeCastError` for anything but a number.
    def as_f : Float64
      as_f? || raise TypeCastError.new("this answer value is not a number")
    end

    # A whole number; nil for anything else (including 1.5).
    def as_i? : Int64?
      as_f?.try { |f| f == f.trunc && f.abs < 9.0e15 ? f.to_i64 : nil }
    end

    # True only for JSON `true` (false for `false` and anything else).
    def as_bool : Bool
      LibWol.wol_info_get_bool(live) == 1
    end

    # The texts of a list of strings (items that are not text are skipped).
    def strings : Array(::String)
      compact_map(&.as_s?)
    end

    # The whole value as JSON text.
    def json : ::String
      Strings.borrow(LibWol.wol_info_get_json(live)) || "null"
    end

    # The whole value decoded into `type` (a `JSON::Serializable`). Raises
    # `JSON::SerializableError` when it does not fit.
    def decode(type : T.class) : T forall T
      T.from_json(json)
    end

    # Writes the JSON text.
    def to_s(io : IO) : Nil
      io << json
    end

    private def live : LibWol::Info*
      raise UsageError.new("Wol::Info was used after its answer was closed") if closed?
      @ptr
    end

    private def child(ptr : LibWol::Info*) : Info?
      ptr.null? ? nil : Info.new(ptr, @root || self)
    end
  end
end
