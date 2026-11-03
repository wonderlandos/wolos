# SPDX-License-Identifier: GPL-2.0-or-later
require "json"
require "uri"
require "./core"
require "./args"

module Wopkg
  # `wopkg shard list|add|update|check` (docs/modules/shards.md, docs/RULES-crystal.md RCR-040 to
  # RCR-044): Crystal shards from the signed WolOS shard list, pinned by their full commit in a
  # project's shard.yml, with a shard.lock that `shards install --frozen --local` accepts with no
  # network. libwopkg (crates/wopkg/src/index/shards.rs) checks the list and writes both files;
  # this file prints the results and fills the shards cache (a git mirror per shard, at the pinned
  # commit) so the install that follows stays offline. `--local` skips that fetch.
  class Shard
    # One shard of the list.
    struct Entry
      include JSON::Serializable

      getter name : String
      getter git : String
      getter version : String
      getter commit : String
      getter license : String
      getter summary : String = ""
      getter crystal : String = ""
      getter dependencies : Array(String) = [] of String
    end

    # `wopkg_shard_index` result.
    struct Listing
      include JSON::Serializable

      getter shards : Array(Entry)
      getter serial : UInt64 = 0
    end

    # A pin that moved or appeared.
    struct Change
      include JSON::Serializable

      getter name : String
      getter from : String
      getter to : String
      getter version : String
    end

    # A shard the lock names.
    struct Locked
      include JSON::Serializable

      getter name : String
      getter git : String
      getter commit : String
      getter version : String
    end

    # `wopkg_shard_pin` result.
    struct Pinned
      include JSON::Serializable

      getter changed : Array(Change)
      getter locked : Array(Locked)
      getter shard_yml : String
      getter shard_lock : String
    end

    # One problem `check` found.
    struct Finding
      include JSON::Serializable

      getter rule : String
      getter line : Int32
      getter file : String
      getter message : String
    end

    # `wopkg_shard_check` result.
    struct Checked
      include JSON::Serializable

      getter? ok : Bool
      getter? index : Bool
      getter findings : Array(Finding)
    end

    USAGE = "wopkg shard list [words] | add <name>... | update [name]... | check   [folder with a /, default: here]"

    def initialize(@core : Core, @a : Args, @out : IO = STDOUT)
    end

    # The exit status: 0 when done, 1 when `check` found problems.
    def run : Int32
      verb = @a.targets[0]? || "list"
      dir, names = Shard.split(@a.targets[1..]? || [] of String)
      case verb
      when "list"
        list(names.join(" "))
      when "add"
        raise UsageError.new("wopkg shard add <name>... [folder]") if names.empty?
        pin(dir, names, update: false)
      when "update"
        pin(dir, names, update: true)
      when "check"
        return check(dir, names)
      else
        raise UsageError.new(USAGE)
      end
      0
    end

    # Words -> (project folder, names). A word with a `/` (or `.`, `..`) is the folder: shard
    # names never hold one.
    def self.split(words : Array(String)) : {String, Array(String)}
      dirs, names = words.partition { |w| w.includes?('/') || w == "." || w == ".." }
      raise UsageError.new("Name one project folder.") if dirs.size > 1
      {File.expand_path(dirs.first? || "."), names}
    end

    private def list(term : String)
      text = @core.shard_index({term: term}.to_json)
      return @out.puts(text) if @a.json?
      l = Listing.from_json(text)
      if l.shards.empty?
        @out.puts "No shard in the WolOS shard list matches “#{term}”."
        return
      end
      l.shards.each do |e|
        @out.puts "#{e.name.ljust(18)} #{e.version.ljust(10)} #{e.license.ljust(14)} #{e.summary}"
      end
    end

    private def pin(dir : String, names : Array(String), update : Bool)
      text = @core.shard_pin({dir: dir, names: names, update: update, fresh: update}.to_json)
      p = Pinned.from_json(text)
      missing = @a.local? ? [] of String : p.locked.reject { |l| Shard.prime(l.git, l.commit) }.map(&.name)
      unless missing.empty?
        raise Failure.new(LibWopkg::WOPKG_ERR_UNAVAILABLE,
          "shard.yml and shard.lock are written, but #{missing.join(", ")} could not be fetched at the pinned commit; run `wopkg shard update` again online.")
      end
      return @out.puts(text) if @a.json?
      if p.changed.empty?
        @out.puts "Every shard is already at the commit the WolOS shard list vouches for."
      end
      p.changed.each do |c|
        from = c.from.empty? ? "" : " (was #{c.from[0, 12]})"
        @out.puts "Pinned #{c.name} #{c.version} at #{c.to[0, 12]}#{from}."
      end
      @out.puts "#{p.shard_lock}: #{p.locked.map(&.name).join(", ")}." unless p.locked.empty?
      @out.puts @a.local? ? "Fetch the shards once online, or run `shards install --frozen` on a machine that may." : "Ready for `shards install --frozen --local`."
    end

    private def check(dir : String, names : Array(String)) : Int32
      raise UsageError.new("wopkg shard check [folder]") unless names.empty?
      text = @core.shard_check({dir: dir, offline: @a.local?}.to_json)
      c = Checked.from_json(text)
      if @a.json?
        @out.puts text
        return c.ok? ? 0 : 1
      end
      c.findings.each { |f| @out.puts "#{File.join(dir, f.file)}:#{f.line}: #{f.rule}: #{f.message}" }
      @out.puts "The WolOS shard list was not read: only RCR-040 and RCR-041 were checked." unless c.index?
      if c.ok?
        @out.puts c.index? ? "shard.yml and shard.lock are pinned and match the WolOS shard list." : "shard.yml and shard.lock are pinned."
      end
      c.ok? ? 0 : 1
    end

    # The shards cache folder: SHARDS_CACHE_PATH, else XDG_CACHE_HOME/shards, else ~/.cache/shards.
    def self.cache_root : String
      ENV["SHARDS_CACHE_PATH"]? || File.join(ENV["XDG_CACHE_HOME"]? || File.join(ENV["HOME"]? || ".", ".cache"), "shards")
    end

    # Where `shards` keeps the mirror of `git`: cache root, then host, then the path ending in `.git`.
    def self.cache_path(git : String, root : String = cache_root) : String
      uri = URI.parse(git)
      path = uri.path
      path += ".git" unless path.ends_with?(".git")
      host = uri.host
      host && !host.empty? ? File.join(root, host, path) : File.join(root, path)
    end

    # Makes the mirror `shards` reads with `--local` hold `commit`: clones it when missing, else
    # fetches; a commit no branch carries any more is fetched by its hash and kept under a ref so
    # git does not prune it. False when the commit could not be had.
    def self.prime(git : String, commit : String, root : String = cache_root) : Bool
      path = cache_path(git, root)
      if Dir.exists?(path)
        git(["-C", path, "fetch", "--quiet", "--prune", "origin"])
      else
        Dir.mkdir_p(File.dirname(path))
        return false unless git(["clone", "--mirror", "--quiet", "--", git, path])
      end
      return true if git(["-C", path, "cat-file", "-e", "#{commit}^{commit}"])
      git(["-C", path, "fetch", "--quiet", "origin", commit]) &&
        git(["-C", path, "update-ref", "refs/wolos/pinned/#{commit}", commit]) &&
        git(["-C", path, "cat-file", "-e", "#{commit}^{commit}"])
    end

    private def self.git(args : Array(String)) : Bool
      Process.run("git", args, env: {"GIT_TERMINAL_PROMPT" => "0"}, output: Process::Redirect::Close, error: Process::Redirect::Close).success?
    rescue File::NotFoundError | IO::Error
      false
    end
  end
end
