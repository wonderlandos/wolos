# SPDX-License-Identifier: BSD-2-Clause
require "json"
require "digest/sha256"
require "file_utils"
require "./toml"
require "./sandbox"

module Wopkg
  # Official recipes (ADR 0021): a `recipe.toml` with pinned sources, an optional lockfile-checked
  # fetch step (network), then build and install steps without network, all in the sandbox, as the
  # person. Results are cached by the recipe's own hash and installed into ~/.local.
  class Recipe
    class Invalid < Exception
    end

    record Source, url : String, sha256 : String, file : String

    # Fetch steps may only run tools that check every download against a lockfile.
    FETCHERS = ["cargo fetch --locked", "go mod download", "dotnet restore --locked-mode", "npm ci --ignore-scripts"]

    getter name : String
    getter version : String
    getter summary : String
    getter license : String
    getter homepage : String
    getter needs : Array(String)
    getter sources : Array(Source)
    getter fetch : Array(String)
    getter build : Array(String)
    getter install : Array(String)
    getter bins : Array(String)
    getter subdir : String
    getter text : String

    protected def self.str(h, k : String, req = true) : String
      v = h[k]?
      return "" if v.nil? && !req
      v.as?(String) || raise Invalid.new("#{k} must be text")
    end

    protected def self.list(h, k : String) : Array(String)
      v = h[k]?
      return [] of String if v.nil?
      arr = v.as?(Array) || raise Invalid.new("#{k} must be a list")
      arr.map { |x| x.as?(String) || raise Invalid.new("#{k} must list text") }
    end

    def self.parse(text : String) : Recipe
      t = begin
        Toml.parse(text)
      rescue e : Toml::ParseError
        raise Invalid.new(e.message || "bad recipe")
      end
      new(t, text)
    end

    def initialize(t : Hash(String, Toml::Value), @text : String)
      @name = Recipe.str(t, "name")
      @version = Recipe.str(t, "version")
      @summary = Recipe.str(t, "summary", false)
      @license = Recipe.str(t, "license")
      @homepage = Recipe.str(t, "homepage", false)
      @needs = Recipe.list(t, "needs")
      raise Invalid.new("name is lowercase letters, digits and -") unless @name.matches?(/\A[a-z0-9][a-z0-9-]{0,63}\z/)
      raise Invalid.new("version is digits, letters, . and +") unless @version.matches?(/\A[0-9A-Za-z.+]{1,32}\z/)
      srcs = t["source"]?.try(&.as?(Array)) || raise Invalid.new("a recipe needs at least one [[source]]")
      @sources = srcs.map do |s|
        h = s.as?(Hash) || raise Invalid.new("[[source]] must be a table")
        url = Recipe.str(h, "url")
        sum = Recipe.str(h, "sha256").downcase
        raise Invalid.new("sources are downloaded over HTTPS only: #{url}") unless url.starts_with?("https://")
        raise Invalid.new("sha256 of #{url} is 64 hex digits") unless sum.matches?(/\A[0-9a-f]{64}\z/)
        file = Recipe.str(h, "file", false)
        file = url.split('/').last.split('?').first if file.empty?
        raise Invalid.new("bad source file name #{file}") unless file.matches?(/\A[A-Za-z0-9._+-]+\z/) && !file.starts_with?('.')
        Source.new(url, sum, file)
      end
      sec = ->(k : String) { t[k]?.try(&.as?(Hash)) || ({} of String => Toml::Value) }
      @fetch = Recipe.list(sec.call("fetch"), "run")
      @build = Recipe.list(sec.call("build"), "run")
      inst = sec.call("install")
      @install = Recipe.list(inst, "run")
      @bins = Recipe.list(inst, "bin")
      @subdir = Recipe.str(sec.call("build"), "dir", false)
      raise Invalid.new("[build] needs run steps") if @build.empty?
      raise Invalid.new("[install] needs bin or run") if @bins.empty? && @install.empty?
      @fetch.each do |f|
        raise Invalid.new("fetch steps run one command: “#{f}”") if f.matches?(/[;&|`$<>]/)
        raise Invalid.new("fetch steps may only run #{FETCHERS.join(", ")}: “#{f}”") unless FETCHERS.any? { |p| f == p || f.starts_with?(p + " ") }
      end
      (@bins).each { |b| raise Invalid.new("bin paths stay inside the build: #{b}") if b.starts_with?('/') || b.split('/').includes?("..") }
      raise Invalid.new("[build] dir stays inside the build") if @subdir.starts_with?('/') || @subdir.split('/').includes?("..")
    end

    # Cache key: the recipe's bytes.
    def key : String
      Digest::SHA256.hexdigest(@text)[0, 16]
    end

    def to_index_entry(sha256 : String) : Hash(String, String)
      {"name" => @name, "version" => @version, "summary" => @summary, "sha256" => sha256}
    end

    # Recipe folders: $WOPKG_RECIPES (colon-separated), then the system's.
    def self.dirs : Array(String)
      (ENV["WOPKG_RECIPES"]?.try(&.split(':')) || [] of String) + ["/usr/share/wopkg/recipes"]
    end

    def self.find(name : String) : {Recipe, String}?
      dirs.each do |d|
        p = "#{d}/#{name}/recipe.toml"
        return {parse(File.read(p)), p} if File.exists?(p)
      end
      nil
    end

    # `index.json` for a recipe folder (published with `index.json.sig`, raw Ed25519).
    def self.index(dir : String) : String
      entries = Dir.children(dir).sort.compact_map do |n|
        p = "#{dir}/#{n}/recipe.toml"
        next unless File.exists?(p)
        text = File.read(p)
        parse(text).to_index_entry(Digest::SHA256.hexdigest(text))
      end
      {"format" => 1, "recipes" => entries}.to_pretty_json + "\n"
    end
  end

  # Runs a recipe: sources through libwopkg (hash-checked), steps in the sandbox, cached output.
  class RecipeBuilder
    getter work : String

    def initialize(@recipe : Recipe, cache : String)
      @work = "#{cache}/recipes/#{@recipe.name}-#{@recipe.version}-#{@recipe.key}"
    end

    def cached? : Bool
      File.exists?("#{@work}/.built")
    end

    def missing_tools : Array(String)
      @recipe.needs.reject { |t| Process.find_executable(t) }
    end

    # Sources for `wopkg_fetch`.
    def fetch_request(dl : String) : String
      {"items" => @recipe.sources.map { |s| {"url" => s.url, "dest" => "#{dl}/#{s.sha256[0, 12]}-#{s.file}", "sha256" => s.sha256} }}.to_json
    end

    def run(core, prefix : String, echo = false, &on_step : String ->) : String
      Sandbox.check_user!
      return "#{@work}/out" if cached?
      m = missing_tools
      raise Recipe::Invalid.new("Building #{@recipe.name} needs #{m.join(", ")}. Install #{m.size == 1 ? "it" : "them"} first.") unless m.empty?
      dl = "#{@work}/../.sources"
      Dir.mkdir_p(dl)
      res = core.fetch(fetch_request(dl))
      res["items"].as_a.each do |it|
        if it["failed"]? || it["message"]?
          raise Recipe::Invalid.new(it["message"]?.try(&.as_s) || "A source could not be downloaded.")
        end
      end
      FileUtils.rm_rf("#{@work}/src")
      Dir.mkdir_p("#{@work}/src")
      Dir.mkdir_p("#{@work}/out")
      @recipe.sources.each do |s|
        f = "#{dl}/#{s.sha256[0, 12]}-#{s.file}"
        if s.file.matches?(/\.(tar\.(gz|xz|zst|bz2)|tgz|crate|zip)\z/)
          st = Process.run("bsdtar", ["-xf", f, "-C", "#{@work}/src", "--no-same-owner", "--no-same-permissions"])
          raise Recipe::Invalid.new("#{s.file} could not be unpacked.") unless st.success?
        else
          File.copy(f, "#{@work}/src/#{s.file}")
        end
      end
      # One top folder (a release tarball): build in it.
      tops = Dir.children("#{@work}/src")
      dir = @recipe.subdir.empty? && tops.size == 1 && File.directory?("#{@work}/src/#{tops[0]}") ? "/work/src/#{tops[0]}" : "/work/src/#{@recipe.subdir}"
      log = "#{@work}/build.log"
      # PREFIX: where files go now (copied to the person's prefix after); TARGET_PREFIX: where they
      # will live, for programs that compile their data path in.
      env = {"PREFIX" => "/work/out", "TARGET_PREFIX" => prefix}
      @recipe.fetch.each { |c| on_step.call("fetch: #{c}"); Sandbox.run(@work, c, true, log, dir, env, echo) || raise Recipe::Invalid.new("Fetching for #{@recipe.name} failed. See #{log}.") }
      (@recipe.build + @recipe.install).each { |c| on_step.call(c); Sandbox.run(@work, c, false, log, dir, env, echo) || raise Recipe::Invalid.new("Building #{@recipe.name} failed. See #{log}.") }
      host_dir = dir.sub("/work", @work)
      Dir.mkdir_p("#{@work}/out/bin")
      @recipe.bins.each do |b|
        src = "#{host_dir}/#{b}"
        raise Recipe::Invalid.new("The build made no #{b}.") unless File.file?(src)
        File.copy(src, "#{@work}/out/bin/#{File.basename(b)}")
        File.chmod("#{@work}/out/bin/#{File.basename(b)}", 0o755)
      end
      File.write("#{@work}/.built", Time.utc.to_unix.to_s)
      "#{@work}/out"
    end

    # Copy the build output into `prefix` (~/.local); returns the files written.
    def self.install(built : String, prefix : String) : Array(String)
      files = [] of String
      Dir.glob("#{built}/**/*", match: :dot_files).sort.each do |p|
        next if File.directory?(p)
        rel = p.lchop(built).lchop('/')
        dest = "#{prefix}/#{rel}"
        Dir.mkdir_p(File.dirname(dest))
        tmp = "#{dest}.wopkg-new"
        File.copy(p, tmp)
        File.rename(tmp, dest)
        files << dest
      end
      files
    end
  end
end
