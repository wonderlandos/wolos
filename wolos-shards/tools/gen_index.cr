# SPDX-License-Identifier: GPL-2.0-or-later
# The wolos-shards index generator (docs/modules/shards.md "The list"): reads curated.yml (the
# shards WolOS has reviewed, each at one full commit), checks every entry against the shard's own
# shard.yml at that commit, and writes public/index.json in format 1 with the next serial and an
# expiry. Why: the list is what `wol shard add` trusts, so every field in it comes from the
# reviewed commit itself, never typed by hand. How:
#   crystal run tools/gen_index.cr -- check            (CI, every pull request: no writes)
#   crystal run tools/gen_index.cr -- build [days]     (writes public/index.json, default 30 days)
#   crystal run tools/gen_index.cr -- sign <key.pem>   (public/index.json.sig, Ed25519, offline key)
require "json"
require "yaml"
require "file_utils"

module ShardIndexGen
  # One reviewed shard: where it is published, the commit, the line the list shows. `fetch` is an
  # optional other place to read the same commit from (a review mirror). `path` is the shard's
  # folder inside the repository `fetch` names, for a shard that still lives in the WolOS source
  # tree: `check` reads it there, `build` refuses it (README.md "Shards still in the WolOS tree").
  struct Curated
    include YAML::Serializable

    getter name : String
    getter git : String
    getter commit : String
    getter summary : String = ""
    getter fetch : String? = nil
    getter path : String? = nil
  end

  # curated.yml.
  struct CuratedList
    include YAML::Serializable

    getter shards : Array(Curated) = [] of Curated
  end

  # A dependency in a shard's own shard.yml.
  struct Dependency
    include YAML::Serializable

    getter git : String? = nil
    getter commit : String? = nil
  end

  # The parts of a shard's own shard.yml the list carries.
  struct ShardYml
    include YAML::Serializable

    getter name : String
    getter version : String
    getter license : String? = nil
    getter crystal : String? = nil
    getter dependencies : Hash(String, Dependency) = {} of String => Dependency
  end

  # One entry of index.json (crates/wopkg/src/index/shards.rs `Entry`).
  struct Entry
    include JSON::Serializable

    getter name : String
    getter git : String
    getter version : String
    getter commit : String
    getter license : String
    getter summary : String
    getter crystal : String
    getter dependencies : Array(String)

    def initialize(@name, @git, @version, @commit, @license, @summary, @crystal, @dependencies)
    end
  end

  # index.json, format 1.
  struct Index
    include JSON::Serializable

    getter format : Int32 = 1
    getter serial : Int64 = 0
    getter expires : Int64 = 0
    getter shards : Array(Entry) = [] of Entry

    def initialize(@serial, @expires, @shards)
    end
  end

  NAME    = /\A[a-z][a-z0-9_-]{0,49}\z/
  GIT     = /\Ahttps:\/\/[A-Za-z0-9.:-]+\/[A-Za-z0-9\/._~-]+\z/
  COMMIT  = /\A[0-9a-f]{40}\z/
  VERSION = /\A[0-9][A-Za-z0-9.-]{0,63}\z/
  PATH    = /\A[A-Za-z0-9][A-Za-z0-9\/._-]{0,199}\z/

  # Runs git; its stdout, or nil when it fails.
  def self.git(args : Array(String)) : String?
    buf = IO::Memory.new
    st = Process.run("git", args, env: {"GIT_TERMINAL_PROMPT" => "0"}, output: buf, error: Process::Redirect::Close)
    st.success? ? buf.to_s : nil
  end

  # The entries for `list`, and every problem found (an empty list of problems means publishable).
  def self.resolve(list : CuratedList, work : String) : {Array(Entry), Array(String)}
    problems = [] of String
    entries = [] of Entry
    by_name = {} of String => Curated
    list.shards.each do |c|
      problems << "#{c.name}: listed twice" if by_name.has_key?(c.name)
      by_name[c.name] = c
    end
    list.shards.each do |c|
      bad = [] of String
      bad << "name is not a shard name" unless c.name.matches?(NAME)
      dot_segment = c.git.split('/').any? { |seg| seg == ".." || seg == "." }
      bad << "git is not an https:// URL without user, query or fragment" if dot_segment || !c.git.matches?(GIT)
      bad << "commit is not 40 lowercase hex digits" unless c.commit.matches?(COMMIT)
      bad << "summary is longer than 200 characters" if c.summary.size > 200
      if folder = c.path
        bad << "path is not a plain relative folder" unless folder.matches?(PATH) && folder.split('/').none? { |seg| seg == ".." || seg == "." }
        bad << "path needs fetch: (the repository that holds the folder)" unless c.fetch
      end
      unless bad.empty?
        bad.each { |b| problems << "#{c.name}: #{b}" }
        next
      end
      mirror = File.join(work, "#{c.name}.git")
      unless git(["clone", "--mirror", "--quiet", "--", c.fetch || c.git, mirror])
        problems << "#{c.name}: #{c.fetch || c.git} cannot be cloned"
        next
      end
      yml = c.path.try { |dir| "#{dir}/shard.yml" } || "shard.yml"
      text = git(["--git-dir", mirror, "show", "#{c.commit}:#{yml}"])
      unless text
        problems << "#{c.name}: commit #{c.commit} is not there or has no #{yml}"
        next
      end
      begin
        y = ShardYml.from_yaml(text)
      rescue e : YAML::ParseException
        problems << "#{c.name}: shard.yml at #{c.commit[0, 12]} does not read: #{e.message}"
        next
      end
      problems << "#{c.name}: shard.yml at that commit says name #{y.name}" unless y.name == c.name
      problems << "#{c.name}: version #{y.version} is not a plain version" unless y.version.matches?(VERSION)
      license = y.license || ""
      problems << "#{c.name}: shard.yml names no license" if license.strip.empty?
      y.dependencies.each do |dep, d|
        pinned = by_name[dep]?
        if pinned.nil?
          problems << "#{c.name}: depends on #{dep}, which is not curated"
        elsif d.commit != pinned.commit
          problems << "#{c.name}: depends on #{dep} at #{d.commit || "no commit"}, the list has #{pinned.commit}"
        end
      end
      entries << Entry.new(c.name, c.git, y.version, c.commit, license, c.summary, y.crystal || "", y.dependencies.keys.sort!)
    end
    {entries.sort_by(&.name), problems}
  end

  def self.main(args : Array(String)) : Int32
    root = File.expand_path("..", __DIR__)
    cmd = args[0]? || "check"
    pub = File.join(root, "public")
    index_path = File.join(pub, "index.json")
    if cmd == "sign"
      key = args[1]? || return usage
      st = Process.run("openssl", ["pkeyutl", "-sign", "-rawin", "-inkey", key, "-in", index_path, "-out", "#{index_path}.sig"], error: Process::Redirect::Inherit)
      return 1 unless st.success? && File.size("#{index_path}.sig") == 64
      puts "Signed #{index_path}."
      return 0
    end
    return usage unless cmd == "check" || cmd == "build"
    list = CuratedList.from_yaml(File.read(File.join(root, "curated.yml")))
    work = File.tempname("wolos-shards")
    Dir.mkdir_p(work)
    begin
      entries, problems = resolve(list, work)
    ensure
      FileUtils.rm_rf(work)
    end
    problems.each { |p| STDERR.puts p }
    return 1 unless problems.empty?
    if cmd == "check"
      puts "#{entries.size} curated shards check."
      return 0
    end
    # A shard still in the WolOS tree is reviewed, but its `git` URL does not hold that commit yet:
    # publishing it would pin people to a commit `shards install` cannot fetch.
    in_tree = list.shards.select(&.path).map(&.name)
    unless in_tree.empty?
      STDERR.puts "Not published while they live in the WolOS tree (README.md): #{in_tree.join(", ")}"
      return 1
    end
    days = (args[1]? || "30").to_i
    last = File.exists?(index_path) ? Index.from_json(File.read(index_path)).serial : 0_i64
    index = Index.new(last + 1, Time.utc.to_unix + days.to_i64 * 86_400, entries)
    Dir.mkdir_p(pub)
    File.write(index_path, index.to_pretty_json + "\n")
    puts "Wrote #{index_path}: serial #{index.serial}, #{entries.size} shards, valid #{days} days. Sign it next."
    0
  end

  def self.usage : Int32
    STDERR.puts "usage: crystal run tools/gen_index.cr -- check | build [days] | sign <key.pem>"
    2
  end
end

exit ShardIndexGen.main(ARGV)
