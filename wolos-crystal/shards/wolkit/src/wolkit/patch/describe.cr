# SPDX-License-Identifier: BSD-2-Clause
# Patch.describe: turns patch stream bytes back into one readable line per op. Specs use it to pin
# what the DSL emits (a "golden patch stream"), and the dev overlay can print a patch with it. It
# reads with bounds checks only; it does not validate the patch the way the engine does.
require "./wire"

module Wolkit
  module Patch
    # The ops of `bytes` as text, for example `CREATE 1 box`, `SET_PROP 1 gap 12.0`, `INSERT 0 1 append`.
    def self.describe(bytes : Bytes) : Array(String)
      raise ArgumentError.new("A patch is at least #{Wire::HEADER} bytes.") if bytes.size < Wire::HEADER
      magic = u32(bytes, 0)
      raise ArgumentError.new("This is not a Wolkit patch (magic #{magic.to_s(16)}).") unless magic == Wire::MAGIC
      count = u32(bytes, 12)
      pool_len = u32(bytes, 16).to_i
      pool = bytes[bytes.size - pool_len, pool_len]
      lines = ["PATCH seq=#{u32(bytes, 8)} ops=#{count} pool=#{pool_len}"]
      at = Wire::HEADER
      count.times do
        break if at + 4 > bytes.size - pool_len
        code = u16(bytes, at)
        words = (u16(bytes, at + 2) & ~Wire::MUST_UNDERSTAND).to_i
        payload = Array(UInt32).new(words - 1) { |i| u32(bytes, at + 4 + i * 4) }
        lines << line(code, payload, pool)
        at += words * 4
      end
      lines
    end

    private def self.line(code : UInt16, w : Array(UInt32), pool : Bytes) : String
      case code
      when Wire::Op::CREATE     then "CREATE #{w[0]} #{kind_name(w[1])}"
      when Wire::Op::INSERT     then "INSERT #{w[0]} #{w[1]} #{w[2] == Wire::APPEND ? "append" : w[2].to_s}"
      when Wire::Op::REMOVE     then "REMOVE #{w[0]}"
      when Wire::Op::MOVE       then "MOVE #{w[0]} #{w[1]} #{w[2] == Wire::APPEND ? "append" : w[2].to_s}"
      when Wire::Op::SET_PROP   then set_prop(w, pool)
      when Wire::Op::SET_TEXT   then "SET_TEXT #{w[0]} #{str(pool, w[1]).inspect}"
      when Wire::Op::SET_STYLE  then "SET_STYLE #{w[0]} #{w[2, w[1].to_i].join(' ')}".rstrip
      when Wire::Op::SET_STATE  then "SET_STATE #{w[0]} #{w[1]}"
      when Wire::Op::SET_KEY    then "SET_KEY #{w[0]} #{w[1].to_u64 | (w[2].to_u64 << 32)}"
      when Wire::Op::BIND_EVENT then "BIND_EVENT #{w[0]} #{w[1].to_s(2)}"
      when Wire::Op::SET_A11Y   then "SET_A11Y #{w[0]} #{role_name(w[1])} #{str(pool, w[2]).inspect} #{str(pool, w[3]).inspect}"
      when Wire::Op::ANIM       then "ANIM #{w[0]} #{prop_name(w[1])} #{f32(w[2])} mode=#{w[3]} param=#{w[4]}"
      when Wire::Op::LIST_RANGE then "LIST_RANGE #{w[0]} count=#{w[1]} height=#{f32(w[2])} overscan=#{w[3]}"
      when Wire::Op::FOCUS      then "FOCUS #{w[0]}"
      when Wire::Op::FREE       then "FREE #{w[0]}"
      else                           "OP#{code} #{w.join(' ')}"
      end
    end

    private def self.set_prop(w : Array(UInt32), pool : Bytes) : String
      name = prop_name(w[1])
      info = Wire::PROPS[name]?
      return "SET_PROP #{w[0]} #{name} #{w[2..].join(' ')}" unless info
      value = case info[:ty]
              when "f32"    then f32(w[2]).to_s
              when "i32"    then w[2].to_i32!.to_s
              when "bool"   then (w[2] == 1).to_s
              when "str"    then str(pool, w[2]).inspect
              when "token"  then "token#{w[2]}"
              when "enum"   then enum_name(info, w[2])
              when "dim"    then "#{%w(auto px %)[w[2]]}:#{f32(w[3])}"
              when "pair"   then "#{f32(w[2])} #{f32(w[3])}"
              when "border" then "#{f32(w[2])} token#{w[3]}"
              else               w[2..].join(' ')
              end
      "SET_PROP #{w[0]} #{name} #{value}"
    end

    private def self.enum_name(info, index : UInt32) : String
      table = info[:en].try { |e| Wire::ENUMS[e]? }
      table.try(&.[index]?) || index.to_s
    end

    private def self.prop_name(id : UInt32) : String
      Wire::PROPS.each { |name, info| return name if info[:id] == id }
      "prop#{id}"
    end

    private def self.kind_name(id : UInt32) : String
      {% for c in Wolkit::Wire::Kind.constants %}
        return {{ c.stringify.downcase }} if id == Wire::Kind::{{ c }}
      {% end %}
      "kind#{id}"
    end

    private def self.role_name(id : UInt32) : String
      {% for c in Wolkit::Wire::Role.constants %}
        return {{ c.stringify.downcase }} if id == Wire::Role::{{ c }}
      {% end %}
      "role#{id}"
    end

    private def self.str(pool : Bytes, ref : UInt32) : String
      return "" if ref == Wire::NO_STR
      len = u32(pool, ref.to_i).to_i
      String.new(pool[ref.to_i + 4, len])
    end

    private def self.f32(bits : UInt32) : Float32
      IO::ByteFormat::LittleEndian.decode(Float32, uint_bytes(bits))
    end

    private def self.uint_bytes(bits : UInt32) : Bytes
      buf = Bytes.new(4)
      IO::ByteFormat::LittleEndian.encode(bits, buf)
      buf
    end

    private def self.u16(buf : Bytes, at : Int32) : UInt16
      buf[at].to_u16 | (buf[at + 1].to_u16 << 8)
    end

    private def self.u32(buf : Bytes, at : Int32) : UInt32
      buf[at].to_u32 | (buf[at + 1].to_u32 << 8) | (buf[at + 2].to_u32 << 16) | (buf[at + 3].to_u32 << 24)
    end
  end
end
