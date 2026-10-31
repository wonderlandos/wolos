# SPDX-License-Identifier: BSD-2-Clause
module Wopkg
  # The TOML subset recipes use: `key = value` with strings ("basic" and 'literal'), integers,
  # booleans and arrays (across lines); `[table]` and `[[array of tables]]`; `#` comments.
  # Anything else is an error with its line number, never a guess.
  module Toml
    alias Value = String | Int64 | Bool | Array(Value) | Hash(String, Value)

    class ParseError < Exception
    end

    def self.parse(text : String) : Hash(String, Value)
      root = {} of String => Value
      cur = root
      lines = text.lines
      i = 0
      while i < lines.size
        n = i + 1
        line = strip_comment(lines[i]).strip
        i += 1
        next if line.empty?
        if line.starts_with?("[[") && line.ends_with?("]]")
          name = key_name(line[2..-3].strip, n)
          arr = (root[name] ||= [] of Value)
          raise ParseError.new("line #{n}: #{name} is not a list of tables") unless arr.is_a?(Array(Value))
          t = {} of String => Value
          arr << t
          cur = t
        elsif line.starts_with?('[') && line.ends_with?(']')
          name = key_name(line[1..-2].strip, n)
          raise ParseError.new("line #{n}: [#{name}] appears twice") if root.has_key?(name)
          t = {} of String => Value
          root[name] = t
          cur = t
        else
          k, eq, v = line.partition('=')
          raise ParseError.new("line #{n}: expected key = value") if eq.empty?
          key = key_name(k.strip, n)
          v = v.strip
          # An array may continue on the following lines until its brackets balance.
          while v.starts_with?('[') && depth(v) > 0 && i < lines.size
            v += " " + strip_comment(lines[i]).strip
            i += 1
          end
          raise ParseError.new("line #{n}: #{key} is set twice") if cur.has_key?(key)
          val, rest = value(v, n)
          raise ParseError.new("line #{n}: unexpected text after the value") unless rest.strip.empty?
          cur[key] = val
        end
      end
      root
    end

    private def self.key_name(k : String, n : Int32) : String
      raise ParseError.new("line #{n}: bad key “#{k}”") unless k.matches?(/\A[A-Za-z0-9_-]+\z/)
      k
    end

    private def self.depth(s : String) : Int32
      d = 0
      q : Char? = nil
      s.each_char do |c|
        if q
          q = nil if c == q
        elsif c == '"' || c == '\''
          q = c
        elsif c == '['
          d += 1
        elsif c == ']'
          d -= 1
        end
      end
      d
    end

    private def self.strip_comment(l : String) : String
      q : Char? = nil
      l.each_char_with_index do |c, i|
        if q
          q = nil if c == q
        elsif c == '"' || c == '\''
          q = c
        elsif c == '#'
          return l[0, i]
        end
      end
      l
    end

    private def self.value(s : String, n : Int32) : {Value, String}
      s = s.lstrip
      if s.starts_with?('"')
        str = String::Builder.new
        j = 1
        while j < s.size
          c = s[j]
          if c == '\\'
            e = s[j + 1]? || raise ParseError.new("line #{n}: unfinished string")
            str << ({'n' => '\n', 't' => '\t', '"' => '"', '\\' => '\\'}[e]? || raise ParseError.new("line #{n}: unknown escape \\#{e}"))
            j += 2
            next
          end
          return {str.to_s, s[(j + 1)..]} if c == '"'
          str << c
          j += 1
        end
        raise ParseError.new("line #{n}: unfinished string")
      elsif s.starts_with?('\'')
        e = s.index('\'', 1) || raise ParseError.new("line #{n}: unfinished string")
        {s[1...e], s[(e + 1)..]}
      elsif s.starts_with?('[')
        arr = [] of Value
        rest = s[1..].lstrip
        until rest.starts_with?(']')
          v, rest = value(rest, n)
          arr << v
          rest = rest.lstrip
          if rest.starts_with?(',')
            rest = rest[1..].lstrip
          elsif !rest.starts_with?(']')
            raise ParseError.new("line #{n}: expected , or ] in a list")
          end
        end
        {arr, rest[1..]}
      elsif m = s.match(/\A(true|false)\b/)
        {m[1] == "true", s[m[0].size..]}
      elsif m = s.match(/\A-?[0-9][0-9_]*/)
        {m[0].delete('_').to_i64, s[m[0].size..]}
      else
        raise ParseError.new("line #{n}: unsupported value “#{s[0, 20]}”")
      end
    end
  end
end
