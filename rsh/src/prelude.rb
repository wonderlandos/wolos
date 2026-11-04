# SPDX-License-Identifier: GPL-2.0-or-later
# rsh: the Ruby half of the shell layer, compiled into the binary. Command lines call
# __rsh_cmd / __rsh_cap (the reader writes those calls); Rsh::Status is $?, Rsh::Failed is what
# an unhandled failure raises. Also: ENV, JSON, Regexp's Ruby side, Process, Etc, require.

class Interrupt < Exception; end unless Object.const_defined?(:Interrupt)
class LoadError < ScriptError; end unless Object.const_defined?(:LoadError)

module Rsh
  # A mistake in a command line itself: a nil value, a file pattern with no match, an unset $NAME.
  class Error < StandardError; end

  # A command failed and the script did not handle it (no if/unless, no ||, not relaxed!).
  class Failed < StandardError
    attr_reader :status, :argv, :stderr

    def initialize(status, argv, stderr)
      @status = status
      @argv = argv
      @stderr = stderr
      how = status.termsig ? "was stopped by signal #{status.termsig}" : "exited #{status.exitstatus}"
      super("#{argv[0]} #{how}")
    end
  end

  class Status
    attr_reader :exitstatus, :statuses, :termsig

    def initialize(code, statuses, sig)
      @exitstatus = code
      @statuses = statuses
      @termsig = sig
    end

    def success?
      @exitstatus == 0
    end

    def signaled?
      !@termsig.nil?
    end

    def to_i
      @exitstatus
    end

    def ==(other)
      other.is_a?(Integer) ? @exitstatus == other : equal?(other)
    end

    def to_s
      "exit #{@exitstatus}"
    end

    def inspect
      "#<Rsh::Status #{@exitstatus}#{@statuses.size > 1 ? " #{@statuses.inspect}" : ''}>"
    end
  end

  @relaxed = false

  def self.relaxed?
    @relaxed
  end

  def self.relaxed=(on)
    @relaxed = on ? true : false
  end

  def self.nullglob=(on)
    __nullglob(on ? true : false)
  end

  def self.__vals(vals)
    vals.map do |x|
      case x
      when String then x
      when Proc then x
      when Array
        x.flatten.map do |e|
          raise Rsh::Error, "a value in a command is nil" if e.nil?
          e.to_s
        end
      when nil then raise Rsh::Error, "a value in a command is nil (an empty argument is \"\")"
      else x.to_s
      end
    end
  end

  # r: [code, statuses, signal, raise?, argv, error tail, interrupted?, output]
  def self.__finish(r)
    st = Status.new(r[0], r[1], r[2])
    __set_status(st)
    raise Interrupt, "interrupted" if r[6]
    raise Failed.new(st, r[4], r[5]) if r[3]
    st
  end

  # Runs one program with these exact arguments (no parsing, no patterns).
  def self.run(*argv)
    __finish(__argv(argv.flatten.map(&:to_s), 0)).success?
  end
end

def relaxed!(on = true)
  Rsh.relaxed = on
end

def __rsh_cmd(i, v, cond = false)
  flags = (cond ? 1 : 0) | (Rsh.relaxed? ? 4 : 0)
  Rsh.__finish(Rsh.__exec(i, Rsh.__vals(v), flags)).success?
end

# A capture gives the output as a String; its status is in $?.
def __rsh_cap(i, v)
  r = Rsh.__exec(i, Rsh.__vals(v), 2 | (Rsh.relaxed? ? 4 : 0))
  Rsh.__finish(r)
  r[7]
end

def __rsh_cd_do(i, v)
  back = Dir.pwd
  __rsh_cmd(i, v)
  begin
    yield
  ensure
    Rsh.__chdir(back)
  end
end

module Kernel
  # Arguments only: system("ls", "-l", dir). One string with spaces would need a shell, which
  # rsh never uses; write the command line itself instead.
  def system(*argv)
    argv = argv.flatten.map(&:to_s)
    if argv.size == 1 && argv[0].include?(" ")
      raise Rsh::Error, "system takes separate arguments: system(\"ls\", \"-l\") or write the command line"
    end
    r = Rsh.__argv(argv, 1)
    st = Rsh.__finish(r)
    st.exitstatus == 127 ? nil : st.success?
  end

  def `(_text)
    raise Rsh::Error, "this capture was built at run time; write it in the script as `...` so values stay single arguments"
  end

  def require(name)
    return false if %w[json set time etc].include?(name.to_s)
    raise LoadError, "cannot load #{name}: rsh has its library built in (json, set, time); load your own files with require_relative"
  end

  def require_relative(path)
    Rsh.__require(path.to_s, true)
  end

  def load(path)
    Rsh.__require(path.to_s, false)
  end
end

class File
  def self.write(path, data, *_opts)
    text = data.to_s
    File.open(path.to_s, "w") { |f| f.write(text) }
    text.bytesize
  end

  def self.readlines(path, *_opts)
    File.open(path.to_s) { |f| f.each_line.to_a }
  end
end

ENV = Object.new
class << ENV
  def [](key)
    Rsh.__getenv(key.to_s)
  end

  def []=(key, value)
    Rsh.__setenv(key.to_s, value.nil? ? nil : value.to_s)
    value
  end

  def fetch(key, *default)
    v = self[key]
    return v if v
    return yield(key) if block_given?
    return default[0] unless default.empty?
    raise KeyError, "key not found: #{key}"
  end

  def key?(key)
    !self[key].nil?
  end
  alias include? key?
  alias has_key? key?

  def delete(key)
    v = self[key]
    Rsh.__setenv(key.to_s, nil)
    v
  end

  def to_h
    Rsh.__env_hash
  end
  alias to_hash to_h

  def each(&block)
    to_h.each(&block)
  end

  def keys
    to_h.keys
  end

  def values
    to_h.values
  end

  def update(hash)
    hash.each { |k, v| self[k] = v }
    self
  end
  alias merge! update

  def inspect
    to_h.inspect
  end
end

module Process
  def self.pid
    Rsh.__pid
  end

  def self.kill(sig, *pids)
    pids.each { |p| Rsh.__kill(sig.to_s, p) }
    pids.size
  end
end

module Etc
  def self.nprocessors
    Rsh.__nproc
  end
end

class Dir
  def self.glob(pattern)
    Array(pattern).flat_map { |p| Rsh.__glob(p) }
  end

  def self.[](*patterns)
    glob(patterns)
  end
end

# ---- JSON ---------------------------------------------------------------------------------------

module JSON
  class ParserError < StandardError; end

  def self.parse(text, symbolize_names: false)
    Rsh.__json_parse(text.to_s, symbolize_names)
  end

  def self.generate(obj)
    obj.to_json
  end

  def self.dump(obj)
    obj.to_json
  end

  def self.pretty_generate(obj, indent = "  ", level = 0)
    pad = indent * (level + 1)
    case obj
    when Hash
      return "{}" if obj.empty?
      body = obj.map { |k, v| "#{pad}#{k.to_s.to_json}: #{pretty_generate(v, indent, level + 1)}" }
      "{\n#{body.join(",\n")}\n#{indent * level}}"
    when Array
      return "[]" if obj.empty?
      body = obj.map { |v| "#{pad}#{pretty_generate(v, indent, level + 1)}" }
      "[\n#{body.join(",\n")}\n#{indent * level}]"
    else
      obj.to_json
    end
  end
end

class Object
  def to_json(*)
    to_s.to_json
  end
end

class String
  def to_json(*)
    Rsh.__json_str(self)
  end
end

class Symbol
  def to_json(*)
    to_s.to_json
  end
end

class Integer
  def to_json(*)
    to_s
  end
end

class Float
  def to_json(*)
    raise JSON::ParserError, "#{self} is not valid JSON" if nan? || infinite?
    to_s
  end
end

class NilClass
  def to_json(*)
    "null"
  end
end

class TrueClass
  def to_json(*)
    "true"
  end
end

class FalseClass
  def to_json(*)
    "false"
  end
end

class Array
  def to_json(*)
    "[#{map(&:to_json).join(',')}]"
  end
end

class Hash
  def to_json(*)
    "{#{map { |k, v| "#{k.to_s.to_json}:#{v.to_json}" }.join(',')}}"
  end
end

# ---- Regexp (the engine is in C++; matching, MatchData and String methods here) -------------------

class MatchData
  attr_reader :regexp, :string

  def initialize(regexp, string, offsets)
    @regexp = regexp
    @string = string
    @offsets = offsets
  end

  def __index(key)
    return key if key.is_a?(Integer)
    i = @regexp.__name_index(key.to_s)
    raise IndexError, "undefined group name reference: #{key}" unless i
    i
  end

  def [](key)
    i = __index(key)
    i += size if i < 0
    return nil if i < 0 || i >= size
    b = @offsets[2 * i]
    return nil if b < 0
    @string.byteslice(b, @offsets[2 * i + 1] - b)
  end

  def size
    @offsets.size / 2
  end
  alias length size

  def byteoffset(key)
    i = __index(key)
    [@offsets[2 * i], @offsets[2 * i + 1]]
  end

  def begin(key)
    b = @offsets[2 * __index(key)]
    b < 0 ? nil : @string.byteslice(0, b).size
  end

  def end(key)
    e = @offsets[2 * __index(key) + 1]
    e < 0 ? nil : @string.byteslice(0, e).size
  end

  def to_a
    (0...size).map { |i| self[i] }
  end

  def captures
    to_a.drop(1)
  end

  def names
    @regexp.names
  end

  def named_captures
    h = {}
    @regexp.names.each { |n| h[n] = self[n] }
    h
  end

  def values_at(*keys)
    keys.map { |k| self[k] }
  end

  def pre_match
    @string.byteslice(0, @offsets[0])
  end

  def post_match
    @string.byteslice(@offsets[1], @string.bytesize - @offsets[1])
  end

  def to_s
    self[0]
  end

  def inspect
    "#<MatchData #{self[0].inspect}>"
  end
end

class Regexp
  IGNORECASE = 1
  EXTENDED = 2
  MULTILINE = 4

  def self.compile(*args)
    new(*args)
  end

  def self.escape(text)
    text.to_s.gsub(/[.*?+^$|(){}\[\]\\\/\-\s]/) { |c| c == "\n" ? "\\n" : c == "\t" ? "\\t" : c == " " ? "\\ " : "\\#{c}" }
  end

  def self.quote(text)
    escape(text)
  end

  def self.union(*parts)
    parts = parts.flatten
    new(parts.map { |p| p.is_a?(Regexp) ? p.source : escape(p) }.join("|"))
  end

  def self.last_match(i = nil)
    i ? $~ && $~[i] : $~
  end

  def match(str, pos = 0)
    if str.nil?
      Rsh.__set_match(nil, [])
      return nil
    end
    str = str.to_s
    bpos = pos <= 0 ? 0 : str[0, pos].to_s.bytesize
    offs = __match(str, bpos)
    md = offs ? MatchData.new(self, str, offs) : nil
    Rsh.__set_match(md, md ? md.captures.first(9) : [])
    if md && block_given?
      yield md
    else
      md
    end
  end

  def match?(str, pos = 0)
    return false if str.nil?
    !__match(str.to_s, pos <= 0 ? 0 : str[0, pos].to_s.bytesize).nil?
  end

  def =~(str)
    md = match(str)
    md && md.begin(0)
  end

  def ===(other)
    other.is_a?(String) || other.is_a?(Symbol) ? match?(other.to_s) : false
  end

  def to_s
    "(?#{options & 1 != 0 ? 'i' : ''}#{options & 4 != 0 ? 'm' : ''}#{options & 2 != 0 ? 'x' : ''}:#{source})"
  end

  def inspect
    "/#{source}/#{options & 4 != 0 ? 'm' : ''}#{options & 1 != 0 ? 'i' : ''}#{options & 2 != 0 ? 'x' : ''}"
  end

  def ==(other)
    other.is_a?(Regexp) && source == other.source && options == other.options
  end
  alias eql? ==

  def casefold?
    options & 1 != 0
  end
end

class String
  alias __rsh_plain_sub sub
  alias __rsh_plain_gsub gsub
  alias __rsh_plain_split split
  alias __rsh_plain_index index

  def =~(other)
    raise TypeError, "wrong argument type String (expected Regexp)" if other.is_a?(String)
    other =~ self
  end

  def match(pattern, pos = 0, &block)
    pattern = Regexp.new(Regexp.escape(pattern)) if pattern.is_a?(String)
    pattern.match(self, pos, &block)
  end

  def match?(pattern, pos = 0)
    pattern = Regexp.new(Regexp.escape(pattern)) if pattern.is_a?(String)
    pattern.match?(self, pos)
  end

  def __rsh_expand(rep, md)
    out = ""
    i = 0
    while i < rep.size
      c = rep[i]
      d = rep[i + 1]
      if c == "\\" && d
        if d >= "0" && d <= "9"
          out << md[d.to_i].to_s
          i += 2
          next
        elsif d == "&"
          out << md[0]
          i += 2
          next
        elsif d == "\\"
          out << "\\"
          i += 2
          next
        elsif d == "k" && rep[i + 2] == "<"
          e = rep.__rsh_plain_index(">", i + 3)
          if e
            out << md[rep[(i + 3)...e]].to_s
            i = e + 1
            next
          end
        end
      end
      out << c
      i += 1
    end
    out
  end

  def __rsh_replace(pattern, rep, global, &block)
    out = ""
    pos = 0
    last = nil
    bytes = bytesize
    while pos <= bytes
      offs = pattern.__match(self, pos)
      break unless offs
      md = MatchData.new(pattern, self, offs)
      last = md
      out << byteslice(pos, offs[0] - pos)
      Rsh.__set_match(md, md.captures.first(9))
      out << if block && rep.nil?
               block.call(md[0]).to_s
             elsif rep.is_a?(Hash)
               rep[md[0]].to_s
             else
               __rsh_expand(rep.to_s, md)
             end
      if offs[1] == offs[0]
        out << byteslice(offs[1], 1).to_s if offs[1] < bytes
        pos = offs[1] + 1
      else
        pos = offs[1]
      end
      break unless global
    end
    out << byteslice(pos, bytes - pos).to_s if pos < bytes
    Rsh.__set_match(last, last ? last.captures.first(9) : [])
    out
  end

  def sub(pattern, rep = nil, &block)
    return __rsh_plain_sub(pattern, rep, &block) unless pattern.is_a?(Regexp)
    __rsh_replace(pattern, rep, false, &block)
  end

  def gsub(pattern, rep = nil, &block)
    return __rsh_plain_gsub(pattern, rep, &block) unless pattern.is_a?(Regexp)
    __rsh_replace(pattern, rep, true, &block)
  end

  def sub!(pattern, rep = nil, &block)
    r = sub(pattern, rep, &block)
    r == self ? nil : replace(r)
  end

  def gsub!(pattern, rep = nil, &block)
    r = gsub(pattern, rep, &block)
    r == self ? nil : replace(r)
  end

  def scan(pattern, &block)
    pattern = Regexp.new(Regexp.escape(pattern)) if pattern.is_a?(String)
    found = []
    pos = 0
    bytes = bytesize
    while pos <= bytes
      offs = pattern.__match(self, pos)
      break unless offs
      md = MatchData.new(pattern, self, offs)
      Rsh.__set_match(md, md.captures.first(9))
      item = md.size > 1 ? md.captures : md[0]
      block ? block.call(item) : found << item
      pos = offs[1] == offs[0] ? offs[1] + 1 : offs[1]
    end
    block ? self : found
  end

  def split(pattern = nil, limit = 0)
    return __rsh_plain_split(pattern, limit) unless pattern.is_a?(Regexp)
    parts = []
    pos = 0
    bytes = bytesize
    search = 0
    while search <= bytes && (limit <= 0 || parts.size < limit - 1)
      offs = pattern.__match(self, search)
      break unless offs
      if offs[1] == offs[0]
        break if offs[0] >= bytes
        if offs[0] == pos
          search = offs[0] + 1
          next
        end
      end
      parts << byteslice(pos, offs[0] - pos)
      (1...(offs.size / 2)).each do |g|
        parts << byteslice(offs[2 * g], offs[2 * g + 1] - offs[2 * g]) if offs[2 * g] >= 0
      end
      pos = offs[1]
      search = offs[1] == offs[0] ? offs[1] + 1 : offs[1]
    end
    parts << byteslice(pos, bytes - pos).to_s
    parts.pop while limit == 0 && !parts.empty? && parts.last.empty?
    parts
  end

  def index(pattern, pos = 0)
    return __rsh_plain_index(pattern, pos) unless pattern.is_a?(Regexp)
    md = pattern.match(self, pos)
    md && md.begin(0)
  end

  def start_with?(*prefixes)
    prefixes.any? do |p|
      if p.is_a?(Regexp)
        md = p.match(self)
        md && md.begin(0) == 0
      else
        p = p.to_s
        self[0, p.size] == p
      end
    end
  end
end

class Symbol
  def =~(other)
    to_s =~ other
  end

  def match(*args)
    to_s.match(*args)
  end
end
