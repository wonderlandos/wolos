# SPDX-License-Identifier: BSD-2-Clause
# PatchWriter: encodes patch stream v1 (interface I10, crates/wolkit/src/scene/patch.rs) into reused
# buffers. One writer lives as long as its window; `clear`, add ops, `finish` hands out the bytes for
# `wolkit_window_apply`. Steady state allocates nothing: the buffers only grow when a patch is bigger
# than any before (plan-wolkit section 4.4).
#
# Layout (little-endian): a 24-byte header, the ops (u16 opcode, u16 MUST_UNDERSTAND|words, then the
# words), the string pool (u32 length, bytes, zero padding to 4) at the very end. A string reference
# is the byte offset of its pool entry.
require "./wire"

module Wolkit
  class PatchWriter
    # Bytes after which a mount should send what it has (the engine takes 1 MiB per apply).
    SOFT_LIMIT = 768 * 1024

    getter op_count = 0
    @ops : Bytes
    @ops_len = 0
    @pool : Bytes
    @pool_len = 0
    @out : Bytes

    def initialize(capacity : Int32 = 16 * 1024)
      @ops = Bytes.new(capacity)
      @pool = Bytes.new(capacity // 4)
      @out = Bytes.new(capacity + capacity // 4 + Wire::HEADER)
    end

    # Start a new patch.
    def clear : Nil
      @ops_len = 0
      @pool_len = 0
      @op_count = 0
    end

    def empty? : Bool
      @op_count == 0
    end

    # Bytes the finished patch would take.
    def size : Int32
      Wire::HEADER + @ops_len + @pool_len
    end

    # True once the patch is big enough that the mount should send it before adding more.
    def full? : Bool
      size >= SOFT_LIMIT
    end

    # The finished patch with sequence number `seq`; valid until the next `clear` or `finish`.
    def finish(seq : UInt32) : Bytes
      total = size
      @out = Bytes.new(total + total // 2) if @out.size < total
      put32(@out, 0, Wire::MAGIC)
      put16(@out, 4, Wire::FORMAT)
      put16(@out, 6, 0_u16)
      put32(@out, 8, seq)
      put32(@out, 12, @op_count.to_u32)
      put32(@out, 16, @pool_len.to_u32)
      put32(@out, 20, 0_u32)
      @ops[0, @ops_len].copy_to(@out[Wire::HEADER, @ops_len])
      @pool[0, @pool_len].copy_to(@out[Wire::HEADER + @ops_len, @pool_len])
      @out[0, total]
    end

    # Put `text` in the pool; the result is the reference to give an op (NO_STR for "").
    def string(text : String) : UInt32
      return Wire::NO_STR if text.empty?
      if text.bytesize > Wire::MAX_STRING
        raise LimitError.new("A text is at most #{Wire::MAX_STRING} bytes; use a list or a file for more.")
      end
      offset = @pool_len
      padded = (text.bytesize + 3) & ~3
      room(:pool, 4 + padded)
      put32(@pool, offset, text.bytesize.to_u32)
      text.to_slice.copy_to(@pool[offset + 4, text.bytesize])
      (text.bytesize...padded).each { |i| @pool[offset + 4 + i] = 0_u8 }
      @pool_len = offset + 4 + padded
      offset.to_u32
    end

    # --- structure

    def create(node : UInt32, kind : UInt32) : Nil
      op(Wire::Op::CREATE, 2)
      word(node)
      word(kind)
    end

    def insert(parent : UInt32, child : UInt32, index : UInt32 = Wire::APPEND) : Nil
      op(Wire::Op::INSERT, 3)
      word(parent)
      word(child)
      word(index)
    end

    def remove(node : UInt32) : Nil
      op(Wire::Op::REMOVE, 1)
      word(node)
    end

    def move(node : UInt32, parent : UInt32, index : UInt32 = Wire::APPEND) : Nil
      op(Wire::Op::MOVE, 3)
      word(node)
      word(parent)
      word(index)
    end

    def free(node : UInt32) : Nil
      op(Wire::Op::FREE, 1)
      word(node)
    end

    def focus(node : UInt32) : Nil
      op(Wire::Op::FOCUS, 1)
      word(node)
    end

    # --- properties (one word per scalar; the engine checks the type against its table)

    def set_f32(node : UInt32, prop : UInt16, value : Float32) : Nil
      prop_head(node, prop, 1)
      float(value)
    end

    # An i32 travels as its two's-complement word (the engine reads it back with `as i32`).
    def set_i32(node : UInt32, prop : UInt16, value : Int32) : Nil
      prop_head(node, prop, 1)
      word(value.to_u32!)
    end

    def set_u32(node : UInt32, prop : UInt16, value : UInt32) : Nil
      prop_head(node, prop, 1)
      word(value)
    end

    def set_bool(node : UInt32, prop : UInt16, value : Bool) : Nil
      prop_head(node, prop, 1)
      word(value ? 1_u32 : 0_u32)
    end

    def set_str(node : UInt32, prop : UInt16, value : String) : Nil
      ref = string(value)
      prop_head(node, prop, 1)
      word(ref)
    end

    # A size: unit 0 auto, 1 px, 2 percent.
    def set_dim(node : UInt32, prop : UInt16, unit : UInt32, value : Float32) : Nil
      prop_head(node, prop, 2)
      word(unit)
      float(value)
    end

    def set_pair(node : UInt32, prop : UInt16, a : Float32, b : Float32) : Nil
      prop_head(node, prop, 2)
      float(a)
      float(b)
    end

    # A border or outline: width and colour token.
    def set_border(node : UInt32, prop : UInt16, width : Float32, token : UInt32) : Nil
      prop_head(node, prop, 2)
      float(width)
      word(token)
    end

    def set_shadow(node : UInt32, prop : UInt16, dx : Float32, dy : Float32, blur : Float32, token : UInt32) : Nil
      prop_head(node, prop, 4)
      float(dx)
      float(dy)
      float(blur)
      word(token)
    end

    # A linear gradient: angle and two or three colour tokens.
    def set_gradient(node : UInt32, prop : UInt16, angle : Float32, tokens : Array(UInt32)) : Nil
      prop_head(node, prop, 5)
      float(angle)
      word(tokens.size.to_u32)
      word(tokens[0])
      word(tokens[1])
      word(tokens.size > 2 ? tokens[2] : 0_u32)
    end

    # transition: property id (0 = all), duration (0 fast, 1 normal, 2 slow), easing.
    def set_transition(node : UInt32, prop : UInt16, property : UInt32, duration : UInt32, easing : UInt32) : Nil
      prop_head(node, prop, 3)
      word(property)
      word(duration)
      word(easing)
    end

    # --- text, style, state, events, accessibility, animation

    def set_text(node : UInt32, text : String) : Nil
      ref = string(text)
      op(Wire::Op::SET_TEXT, 2)
      word(node)
      word(ref)
    end

    def set_style(node : UInt32, classes : Array(UInt32)) : Nil
      raise LimitError.new("A part takes at most #{Wire::MAX_CLASSES} style classes.") if classes.size > Wire::MAX_CLASSES
      op(Wire::Op::SET_STYLE, 2 + classes.size)
      word(node)
      word(classes.size.to_u32)
      classes.each { |c| word(c) }
    end

    def set_state(node : UInt32, mask : UInt32) : Nil
      op(Wire::Op::SET_STATE, 2)
      word(node)
      word(mask)
    end

    def set_key(node : UInt32, key : UInt64) : Nil
      op(Wire::Op::SET_KEY, 3)
      word(node)
      word((key & 0xffff_ffff_u64).to_u32)
      word((key >> 32).to_u32)
    end

    def bind_event(node : UInt32, mask : UInt32) : Nil
      op(Wire::Op::BIND_EVENT, 2)
      word(node)
      word(mask)
    end

    def set_a11y(node : UInt32, role : UInt32, name : String, description : String = "") : Nil
      name_ref = string(name)
      desc_ref = string(description)
      op(Wire::Op::SET_A11Y, 4)
      word(node)
      word(role)
      word(name_ref)
      word(desc_ref)
    end

    # Animate a number property to `target` on the engine clock: mode 0 tween, 1 spring.
    def anim(node : UInt32, prop : UInt16, target : Float32, mode : UInt32, param : UInt32) : Nil
      op(Wire::Op::ANIM, 5)
      word(node)
      word(prop.to_u32)
      float(target)
      word(mode)
      word(param)
    end

    def list_range(node : UInt32, count : UInt32, height : Float32, overscan : UInt32) : Nil
      op(Wire::Op::LIST_RANGE, 4)
      word(node)
      word(count)
      float(height)
      word(overscan)
    end

    private def prop_head(node : UInt32, prop : UInt16, arity : Int32) : Nil
      op(Wire::Op::SET_PROP, 2 + arity)
      word(node)
      word(prop.to_u32)
    end

    # Start an op of `payload` words (the header word is added).
    private def op(code : UInt16, payload : Int32) : Nil
      room(:ops, 4 + payload * 4)
      put16(@ops, @ops_len, code)
      put16(@ops, @ops_len + 2, (payload + 1).to_u16 | Wire::MUST_UNDERSTAND)
      @ops_len += 4
      @op_count += 1
    end

    private def word(value : UInt32) : Nil
      put32(@ops, @ops_len, value)
      @ops_len += 4
    end

    # Grow a buffer so `extra` more bytes fit.
    private def room(which : Symbol, extra : Int32) : Nil
      if which == :ops
        return if @ops_len + extra <= @ops.size
        grown = Bytes.new((@ops_len + extra) * 2)
        @ops[0, @ops_len].copy_to(grown[0, @ops_len])
        @ops = grown
      else
        return if @pool_len + extra <= @pool.size
        grown = Bytes.new((@pool_len + extra) * 2)
        @pool[0, @pool_len].copy_to(grown[0, @pool_len])
        @pool = grown
      end
    end

    private def float(value : Float32) : Nil
      IO::ByteFormat::LittleEndian.encode(value, @ops[@ops_len, 4])
      @ops_len += 4
    end

    private def put16(buf : Bytes, at : Int32, value : UInt16) : Nil
      buf[at] = (value & 0xff).to_u8
      buf[at + 1] = (value >> 8).to_u8
    end

    private def put32(buf : Bytes, at : Int32, value : UInt32) : Nil
      buf[at] = (value & 0xff).to_u8
      buf[at + 1] = ((value >> 8) & 0xff).to_u8
      buf[at + 2] = ((value >> 16) & 0xff).to_u8
      buf[at + 3] = (value >> 24).to_u8
    end
  end
end
