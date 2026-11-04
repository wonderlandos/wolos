# SPDX-License-Identifier: GPL-2.0-or-later
require "./pins"
require "json"
require "http/client"
require "./core"
require "./args"
require "./sources"
require "./ui"
require "./flatpak"
require "./aur"
require "./aur_guard"
require "./aur_hook"
require "./scan"
require "./recipe"
require "./completions"
require "./shard"

module Wopkg
  # The commands. Every system change is one wold call through libwopkg; everything else runs as
  # the person. `--json` prints libwopkg's documents unchanged.
  class App
    getter core : Core
    @a : Args

    # The part of the library's JSON configuration (WOPKG_CONFIG) the tool itself reads.
    struct Config
      include JSON::Serializable

      getter home : String? = nil
      getter cache_dir : String? = nil
      getter state_dir : String? = nil
    end

    # A recipe's install manifest (`recipe_install` writes it).
    struct Manifest
      include JSON::Serializable

      getter version : String
      getter files : Array(String)
    end

    # The assets of a GitHub release.
    struct Release
      include JSON::Serializable

      struct Asset
        include JSON::Serializable

        getter name : String
        getter browser_download_url : String
      end

      getter assets : Array(Asset) = [] of Asset
    end

    def initialize(@a : Args, config = ENV["WOPKG_CONFIG"]? || "{}")
      @config = Config.from_json(config)
      @core = Core.new(config)
    end

    # Root's home is in the sealed, read-only slot: its cache and state live under /var, as
    # libwopkg's (QA-STORE-62; context.rs ROOT_CACHE, ROOT_STATE).
    ROOT_CACHE = "/var/cache/wopkg"
    ROOT_STATE = "/var/lib/wolos/wopkg/state"

    # /proc/self belongs to the process's effective user (no FFI outside the ffi modules, RCR-010).
    def self.root? : Bool
      File.info("/proc/self").owner_id == "0"
    rescue File::Error
      false
    end

    def home : String
      @config.home || ENV["HOME"]?.presence || (App.root? ? "/root" : raise ArgumentError.new("wopkg needs a home folder."))
    end

    def cache : String
      @config.cache_dir || App.dir(ENV["XDG_CACHE_HOME"]?, App.root?, ROOT_CACHE) { "#{home}/.cache/wopkg" }
    end

    def state : String
      @config.state_dir || App.dir(ENV["XDG_STATE_HOME"]?, App.root?, ROOT_STATE) { "#{home}/.local/state/wopkg" }
    end

    # The XDG folder's wopkg directory when set, else root's /var one, else the home's.
    def self.dir(xdg : String?, root : Bool, admin : String, & : -> String) : String
      if x = xdg.presence
        "#{x}/wopkg"
      elsif root
        admin
      else
        yield
      end
    end

    def prefix : String
      "#{home}/.local"
    end

    def say(s : String)
      STDOUT.puts s unless @a.json?
    end

    def note(s : String)
      STDERR.puts s
    end

    def emit(j)
      STDOUT.puts j.to_json if @a.json?
    end

    def run : Int32
      case @a.verb
      when "help"        then help
      when "version"     then say "wopkg #{VERSION} (libwopkg #{Core.version})"
      when "__fetch"     then bench_fetch
      when "__complete"  then complete_names
      when "completions" then completions
      when "sync"        then sync
      when "search"      then search
      when "info"        then info
      when "list"        then list
      when "sources"     then sources
      when "install"     then @a.targets.each { |t| install(t) }
      when "remove"      then @a.targets.each { |t| remove(t) }
      when "update"      then update
      when "history"     then history
      when "pin"         then pin(true)
      when "unpin"       then pin(false)
      when "rollback"    then rollback(@a.targets.first)
      when "channel"     then channel
      when "pause"       then pause
      when "resume"      then File.delete?("#{state}/pause"); say "Automatic system updates are back on."
      when "appimage"    then appimage
      when "aur"         then aur_cmd
      when "audit"       then audit
      when "recipe"      then recipe_cmd
      when "shard"       then return Shard.new(core, @a).run
      end
      0
    end

    # Benchmark helper: download URLs into a folder in parallel (`wopkg __fetch <dir> <url>...`).
    def bench_fetch
      dir = @a.targets.first
      items = @a.targets[1..].map { |u| {"url" => u, "dest" => "#{dir}/#{u.split('/').last}"} }
      res = core.fetch({"items" => items}.to_json, UI.progress("download"))
      bad = res["items"].as_a.count { |i| i["failed"]? }
      say "#{items.size - bad} downloaded, #{bad} failed."
    end

    def help
      say <<-H
        wopkg: apps and packages from every source, one command.

          wopkg search <words>      (-Ss)  find apps in WolOS, Arch, Flathub, AppImage, recipes, community packages
          wopkg info <name>         (-Si)  versions and sources of one app
          wopkg install <name>...   (-S)   install from the best source (--from flatpak|aur|appimage|recipe|<repo>)
          wopkg remove <name>...    (-R)   remove it
          wopkg update              (-Syu) WolOS, system packages, Flatpaks, AppImages, .wol apps, recipes
                --check (-Qu)              only list what would change
                --to <version>             move to a WolOS release (1.1, 2.0)
                --only <kinds>             e.g. --only flatpak,appimage
          wopkg sync                (-Sy)  refresh the search index
          wopkg list                (-Q)   what is installed
          wopkg history | rollback <id|system>
          wopkg pin|unpin <name>           keep a package at its version through updates
          wopkg channel [stable] | pause [days] | resume
          wopkg appimage add <url|file> | wopkg aur review|update <pkg> | wopkg audit <pkg> | wopkg recipe build|install|remove <name>
          wopkg completions zsh|bash|fish|pwsh
          wopkg shard list [words] | add <name>... | update [name]... | check   [folder] (default: here)

        Options: -y (do not ask), --json (for tools).
        H
    end

    # What a shell asks while the person types; it never prints an error into the prompt.
    def complete_names
      STDOUT.puts core.complete(@a.targets.first? || "")
    rescue Failure | IO::Error
      nil
    end

    def completions
      shell = @a.targets.first? || "zsh"
      s = Completions.for(shell) || raise UsageError.new("Completions exist for zsh, bash, fish and pwsh.")
      STDOUT.print s
    end

    def repos : Array(Sources::Repo)
      conf = ENV["WOPKG_PACMAN_CONF"]? || "/etc/pacman.conf"
      File.exists?(conf) ? Sources.pacman_repos(File.read(conf)) : [] of Sources::Repo
    end

    def index_request : String
      Sources.request(repos, Sources.flatpak_appstream(home), @a.local?, @a.only)
    end

    def sync
      t = Time.instant
      res = core.index_update(index_request, UI.progress("index"))
      return emit(res) if @a.json?
      total = 0
      res["sources"].as_a.each do |s|
        total += s["records"].as_i
        err = s["error"]?.try(&.as_s)
        mark = s["rebuilt"].as_bool ? "updated" : "unchanged"
        say "  #{s["name"].as_s.ljust(16)} #{s["records"].as_i.to_s.rjust(7)}  #{err ? "#{mark} · #{err}" : mark}"
      end
      say "Index ready: #{total} packages and apps from #{res["sources"].as_a.size} sources in #{(Time.instant - t).total_milliseconds.round.to_i} ms."
    end

    def ensure_index
      dir = cache
      return if Dir.exists?("#{dir}/index") && !Dir.children("#{dir}/index").empty?
      note "Building the search index for the first time..."
      core.index_update(index_request, UI.progress("index"))
    end

    def search
      ensure_index
      res = core.search(@a.targets.join(" "), 30)
      return emit(res) if @a.json?
      hits = res["results"].as_a
      if hits.empty?
        say "Nothing found for “#{@a.targets.join(" ")}”."
      else
        hits.reverse_each { |h| say UI.search_line(h) }
      end
    end

    def info
      ensure_index
      res = core.info(@a.targets.first)
      return emit(res) if @a.json?
      say UI.bold(res["name"].as_s)
      say "  #{res["summary"].as_s}"
      res["sources"].as_a.each do |s|
        say "  #{UI.source_label(s).ljust(28)} #{s["version"].as_s.ljust(18)} #{s["id"].as_s}#{s["installed"].as_bool ? "  [installed]" : ""}"
      end
    end

    def list
      res = core.installed
      return emit(res) if @a.json?
      {"pacman" => "System packages", "flatpak" => "Flatpak", "wol" => "WolOS apps", "recipe" => "Recipes"}.each do |k, title|
        v = res[k].as_a
        say "#{title} (#{v.size}): #{v.first(12).map(&.as_s).join(", ")}#{v.size > 12 ? ", ..." : ""}" unless v.empty?
      end
      a = res["appimage"].as_a
      say "AppImages (#{a.size}): #{a.map(&.["name"].as_s).join(", ")}" unless a.empty?
    end

    def sources
      Sources::IndexRequest.from_json(index_request).sources.each do |s|
        say "#{s.name.ljust(16)} #{s.kind.ljust(9)} #{s.location}"
      end
    end

    # The source to install `name` from: --from, else the best one.
    def pick(hit : JSON::Any) : JSON::Any
      srcs = hit["sources"].as_a
      if f = @a.from
        srcs.find { |s| s["kind"].as_s == f || s["repo"].as_s == f } ||
          raise Failure.new(6, "#{hit["name"].as_s} is not available from #{f}.")
      else
        srcs.reject { |s| s["kind"].as_s == "aur" }.first? || srcs.first
      end
    end

    def record(source, action, name, from = nil, to = nil, undo = nil)
      e = {"source" => source, "action" => action, "name" => name, "from" => from, "to" => to, "undo" => undo}
      core.record(e.to_json)
    rescue Failure
      nil
    end

    def install(name : String)
      return install_file(name) if File.file?(name)
      ensure_index
      hit = core.info(name)
      s = pick(hit)
      kind = s["kind"].as_s
      id = s["id"].as_s
      if s["installed"].as_bool
        return say "#{hit["name"].as_s} is already installed (#{UI.source_label(s)})."
      end
      note "#{UI.bold(hit["name"].as_s)} #{s["version"].as_s}: #{hit["summary"].as_s}"
      if kind == "aur"
        note "Community packages are not reviewed by WolOS: anyone can publish there. Read what it does before you build it."
      end
      return say "Not installed." unless UI.confirm("Install it from #{UI.source_label(s)}?", @a.yes? && kind != "aur")
      case kind
      when "pacman"
        core.system_call({"call" => "package.change", "verb" => "install", "names" => [id]}.to_json)
      when "flatpak"
        Flatpak.remote_add_argv(s["repo"].as_s).try { |a| Flatpak.run(a) }
        unsigned = Flatpak.unsigned_remotes(Flatpak.capture(Flatpak::REMOTES_ARGV) || "")
        raise Failure.new(5, "The #{s["repo"].as_s} remote does not check signatures, so nothing was installed from it.") if unsigned.includes?(s["repo"].as_s)
        Flatpak.run(Flatpak.install_argv(s["repo"].as_s, id)) || raise Failure.new(3, "Flatpak could not install #{hit["name"].as_s}.")
        record("flatpak", "install", id, nil, s["version"].as_s, {"type" => "run", "argv" => Flatpak.remove_argv(id)})
      when "appimage"
        url = appimage_release_url(id.lchop("gh:"))
        r = core.appimage_install({"url" => url, "name" => hit["name"].as_s}.to_json, UI.progress(hit["name"].as_s))
        note "Signed by its developer; future updates must carry the same key." if r["signed"].as_bool
      when "recipe"
        recipe_install(id)
      when "wol"
        Process.run("wol", ["install", id], output: STDOUT, error: STDERR).success? || raise Failure.new(3, "#{hit["name"].as_s} was not installed.")
      when "aur"
        aur_install(id)
      end
      say "Installed #{hit["name"].as_s}."
    end

    def install_file(path : String)
      if path.ends_with?(".AppImage") || path.ends_with?(".appimage")
        r = core.appimage_install({"path" => File.expand_path(path)}.to_json)
        return say "Installed #{r["app"]["name"].as_s} (AppImage) in ~/Applications."
      end
      if path.ends_with?(".pkg.tar.zst")
        note "#{File.basename(path)} was built on this computer and is not reviewed by WolOS."
        return say "Not installed." unless UI.confirm("Install it for everyone?", false)
        core.system_call({"call" => "package.install-file", "path" => File.expand_path(path)}.to_json)
        return say "Installed #{File.basename(path)}."
      end
      if path.ends_with?(".wol")
        Process.run("wol", ["install", path], output: STDOUT, error: STDERR).success? || raise Failure.new(3, "The app was not installed.")
        return
      end
      raise UsageError.new("wopkg installs .AppImage, .wol and .pkg.tar.zst files.")
    end

    # Newest `*.AppImage` asset of a GitHub repo (x86_64 when there are several).
    def appimage_release_url(repo : String) : String
      raise Failure.new(1, "“#{repo}” is not a GitHub repository.") unless repo.matches?(/\A[\w.-]+\/[\w.-]+\z/)
      client = HTTP::Client.new(URI.parse("https://api.github.com"))
      client.connect_timeout = 10.seconds
      client.read_timeout = 30.seconds
      r = client.get("/repos/#{repo}/releases/latest", headers: HTTP::Headers{"Accept" => "application/vnd.github+json", "User-Agent" => "wopkg"})
      raise Failure.new(3, "GitHub has no release for #{repo}.") unless r.status_code == 200
      release = begin
        Release.from_json(r.body)
      rescue JSON::Error
        raise Failure.new(3, "GitHub's answer about #{repo} could not be read.")
      end
      assets = release.assets.select(&.name.downcase.ends_with?(".appimage"))
      a = assets.find { |x| x.name.includes?("x86_64") || x.name.includes?("amd64") } || assets.first? ||
          raise Failure.new(6, "The latest #{repo} release has no AppImage.")
      a.browser_download_url
    end

    def remove(name : String)
      inst = core.installed
      if inst["appimage"].as_a.any? { |x| x["name"].as_s == name }
        return say "Not removed." unless UI.confirm("Remove #{name} (AppImage)?", @a.yes?)
        id = core.appimage_remove(name)
        return say "Removed #{name}. Undo: wopkg rollback #{id}"
      end
      hit = begin
        core.info(name)
      rescue Failure
        nil
      end
      srcs = hit ? hit["sources"].as_a.select(&.["installed"].as_bool) : [] of JSON::Any
      s = srcs.find { |x| @a.from.nil? || x["kind"].as_s == @a.from } || raise Failure.new(6, "#{name} is not installed.")
      shown = hit ? hit["name"].as_s : name
      return say "Not removed." unless UI.confirm("Remove #{shown} (#{UI.source_label(s)})?", @a.yes?)
      id = s["id"].as_s
      case s["kind"].as_s
      when "flatpak"
        Flatpak.run(Flatpak.remove_argv(id)) || raise Failure.new(3, "Flatpak could not remove #{id}.")
        record("flatpak", "remove", id, s["version"].as_s, nil, {"type" => "run", "argv" => Flatpak.install_argv(s["repo"].as_s, id)})
      when "recipe"
        recipe_remove(id)
      when "wol"
        Process.run("wol", ["remove", id], output: STDOUT, error: STDERR)
      else
        core.system_call({"call" => "package.change", "verb" => "remove", "names" => [id]}.to_json)
      end
      say "Removed #{shown}."
    end

    def want?(kind : String) : Bool
      (o = @a.only).nil? || o.includes?(kind)
    end

    # One command for everything: WolOS release, system packages, Flatpaks, AppImages, .wol, recipes.
    def pin(on : Bool)
      name = @a.targets.first? || raise UsageError.new("wopkg #{on ? "pin" : "unpin"} <package>")
      raise UsageError.new("#{name} is not a package name.") unless Aur.valid?(name)
      Pins.set("#{state}/pins", name, on)
      say on ? "#{name} stays at its version." : "#{name} updates again."
    end

    # One installed app, from the source it came from.
    def update_one(name : String)
      ensure_index
      hit = core.info(name)
      s = hit["sources"].as_a.find(&.["installed"].as_bool) || raise Failure.new(6, "#{name} is not installed.")
      id = s["id"].as_s
      raise Failure.new(5, "#{name} is pinned. Run wopkg unpin #{name} first.") if Pins.read("#{state}/pins").includes?(id)
      case s["kind"].as_s
      when "pacman"
        core.system_call({"call" => "package.change", "verb" => "update", "names" => [id]}.to_json)
      when "flatpak"
        c = Flatpak.capture(Flatpak.commit_argv(id)).try(&.strip)
        Flatpak.run(Flatpak.update_argv([id])) || raise Failure.new(3, "Flatpak could not update #{name}.")
        record("flatpak", "update", id, c, nil, c ? {"type" => "run", "argv" => Flatpak.rollback_argv(id, c)} : nil)
      else
        raise Failure.new(6, "#{name} updates with wopkg update.")
      end
      say "Updated #{name}."
    end

    # A refresh that fails leaves the old index; the update goes on with it.
    private def refresh_index : Nil
      core.index_update(index_request)
    rescue Failure
      nil
    end

    # The WolOS release plan, or nil when the system service cannot say.
    private def release_plan : JSON::Any?
      core.release_plan({"to" => @a.to}.compact.to_json)
    rescue Failure
      nil
    end

    def update
      if (one = @a.targets.first?) && @a.to.nil?
        return update_one(one)
      end
      summary = [] of String
      failed = nil.as(Failure?)
      pins = Pins.read("#{state}/pins")
      refresh_index if @a.only.nil? || want?("pacman")
      if want?("system")
        plan = release_plan
        if plan
          case plan["reason"].as_s
          when "available"
            t = plan["target"]
            summary << "WolOS #{t["version"].as_s} (from #{plan["current"].as_s})"
            if !@a.check? && UI.confirm("Update WolOS to #{t["version"].as_s}? It installs next to the running system and starts on restart.", @a.yes?)
              core.system_call({"call" => "system.update", "manifest" => t["manifest"].as_s, "signature" => t["signature"].as_s}.to_json)
              record("system", "update", "wolos", plan["current"].as_s, t["version"].as_s, {"type" => "system", "note" => "boot the previous system"})
            end
          when "major-needs-choice"
            note "WolOS #{plan["newest"]?.try(&.as_s)} is out. Move to it with `wopkg update --to #{plan["newest"]?.try(&.as_s)}`."
          when "staged"
            note "WolOS #{plan["newest"]?.try(&.as_s)} is rolling out and reaches this computer soon."
          end
        end
      end
      if want?("pacman") && @a.to.nil?
        if @a.check?
          buf = IO::Memory.new
          begin
            Process.run("pacman", ["-Qu"], output: buf)
          rescue IO::Error
            # No pacman here: nothing to count.
          end
          n = buf.to_s.lines.size
          summary << "#{n} system package#{n == 1 ? "" : "s"}" if n > 0
        else
          begin
            core.system_call((pins.empty? ? {"call" => "package.change", "verb" => "update", "names" => [] of String} : {"call" => "package.update-holding", "hold" => pins}).to_json)
            summary << "system packages"
          rescue e : Failure
            if e.code == 3 && @a.only.nil?
              # No system service here: other sources can still update.
            else
              failed = e
            end
          end
        end
      end
      if want?("flatpak") && @a.to.nil? && (ids = Flatpak.capture(Flatpak::UPDATES_ARGV).try { |o| Flatpak.ids(o) }) && !ids.empty?
        ids -= pins
        summary << "#{ids.size} Flatpak app#{ids.size == 1 ? "" : "s"}"
        unless @a.check?
          commits = ids.to_h { |i| {i, Flatpak.capture(Flatpak.commit_argv(i)).try(&.strip)} }
          if !ids.empty? && Flatpak.run(pins.empty? ? Flatpak.update_argv : Flatpak.update_argv(ids))
            commits.each { |i, c| record("flatpak", "update", i, c, nil, c ? {"type" => "run", "argv" => Flatpak.rollback_argv(i, c)} : nil) }
          end
        end
      end
      if want?("appimage") && @a.to.nil?
        r = core.appimage_update({"check_only" => @a.check?, "adopt" => true}.to_json, UI.progress("AppImage"))
        r["apps"].as_a.each do |x|
          case x["state"].as_s
          when "updated"   then summary << "#{x["name"].as_s} (AppImage, #{UI.bytes(x["fetched_bytes"].as_i64)} downloaded)"
          when "available" then summary << "#{x["name"].as_s} (AppImage)"
          when "failed"    then note "#{x["name"].as_s}: #{x["message"].as_s}"
          end
        end
      end
      if want?("wol") && @a.to.nil? && Process.find_executable("wol")
        Process.run("wol", ["update", "apps"] + (@a.check? ? [] of String : ["--yes"]), output: STDERR, error: STDERR)
      end
      if want?("recipe") && @a.to.nil?
        recipe_updates.each do |rname|
          summary << "#{rname} (recipe)"
          recipe_install(rname) unless @a.check?
        end
      end
      if (n = aur.installed.size) > 0 && !@a.json? && (@a.only.nil? || want?("aur"))
        note "#{n} community package#{n == 1 ? "" : "s"} built here: their updates never run on their own. Read and update them with `wopkg aur update`."
      end
      if @a.json?
        emit({"changes" => summary, "checked_only" => @a.check?})
      elsif !summary.empty?
        say "#{@a.check? ? "Updates available" : "Updated"}: #{summary.join(", ")}."
      elsif failed.nil?
        say "Everything is up to date."
      end
      raise failed if failed
    end

    def history
      h = core.history
      return emit(h) if @a.json?
      h["entries"].as_a.last(30).each do |e|
        t = Time.unix(e["time"].as_i64).to_local.to_s("%Y-%m-%d %H:%M")
        undone = e["undone"].as_bool ? "  (undone)" : ""
        ver = [e["from"]?.try(&.as_s?), e["to"]?.try(&.as_s?)].compact.map { |v| v.size > 16 ? v[0, 12] : v }.join(" -> ")
        say "#{e["id"].as_s.rjust(4)}  #{t}  #{e["action"].as_s.ljust(8)} #{e["source"].as_s.ljust(8)} #{e["name"].as_s} #{ver}#{undone}"
      end
    end

    def rollback(id : String)
      if id == "system"
        return say "Nothing changed." unless UI.confirm("Start the previous WolOS on the next restart?", @a.yes?)
        core.system_call({"call" => "system.rollback"}.to_json)
        return say "The previous WolOS starts on the next restart."
      end
      e = core.rollback(id)
      if (u = e["undo"]?) && u["type"]? == "run"
        argv = u["argv"].as_a.map(&.as_s)
        Process.run(argv[0], argv[1..], output: STDERR, error: STDERR).success? || raise Failure.new(3, "The undo did not finish.")
      elsif u && u["type"]? == "system"
        core.system_call({"call" => "system.rollback"}.to_json)
      end
      say "Undone: #{e["action"].as_s} of #{e["name"].as_s}."
    end

    # The channels a person can follow in this release (shared::packages::OFFERED): stable only
    # in 0.13 (QA-STORE-68). A file naming another reads as stable, as `wol update channel` does.
    OFFERED = %w(stable)

    def channel
      if c = @a.targets.first?
        raise UsageError.new("WolOS offers the #{OFFERED.join(", ")} channel only.") unless OFFERED.includes?(c)
        core.system_call({"call" => "system.channel", "channel" => c}.to_json)
        say "This computer follows the #{c} channel."
      else
        text = begin
          File.read("/etc/wolos/channel")
        rescue File::Error
          nil
        end
        say App.channel_of(text)
      end
    end

    def self.channel_of(text : String?) : String
      w = text.try(&.strip) || ""
      OFFERED.includes?(w) ? w : OFFERED.first
    end

    def pause
      days = (@a.targets.first? || "7").to_i? || raise UsageError.new("Pause for a number of days, e.g. `wopkg pause 7`.")
      raise UsageError.new("Pause for 1 to 35 days.") unless (1..35).includes?(days)
      Dir.mkdir_p(state)
      File.write("#{state}/pause", (Time.utc + days.days).to_unix.to_s)
      say "Automatic WolOS updates are paused for #{days} days. Security fixes still install."
    end

    def appimage
      sub = @a.targets.first? || "list"
      case sub
      when "add"
        t = @a.targets[1]? || raise UsageError.new("wopkg appimage add <url or file>")
        r = t.starts_with?("https://") ? core.appimage_install({"url" => t}.to_json, UI.progress("AppImage")) : core.appimage_install({"path" => File.expand_path(t)}.to_json)
        say "Installed #{r["app"]["name"].as_s}#{r["signed"].as_bool ? " (signed)" : ""}."
      when "update"
        @a.only = ["appimage"]
        update
      else
        core.installed["appimage"].as_a.each { |x| say "#{x["name"].as_s.ljust(24)} #{x["update"]?.try(&.as_s?) ? "updates" : "no update info"}#{x["signed"].as_bool ? " · signed" : ""}" }
      end
    end

    def aur
      Aur.new(cache, state)
    end

    def aur_cmd
      sub, pkg = @a.targets[0]?, @a.targets[1]?
      case sub
      when "review"
        pkg || raise UsageError.new("wopkg aur review <package>")
        aur.checkout(pkg) || raise Failure.new(6, "#{pkg} is not in the community package collection.")
        text = aur.review_text(pkg, AurGuard.head(aur.repo(pkg)))
        STDOUT.puts(text.empty? ? "Nothing changed since the version you installed." : text)
      when "update"
        (pkg ? [pkg] : aur.installed).each { |n| aur_install(n, true) }
        say "No community packages built by wopkg yet." if pkg.nil? && aur.installed.empty?
      else
        raise UsageError.new("wopkg aur review <package> | wopkg aur update [package]")
      end
    end

    # What a community package's files say, read and never run. Nothing here builds or installs.
    def audit
      pkg = @a.targets.first? || raise UsageError.new("wopkg audit <package>")
      raise UsageError.new("#{pkg} is not a package name.") unless Aur.valid?(pkg)
      aur.checkout(pkg) || raise Failure.new(6, "#{pkg} is not in the community package collection.")
      dir = aur.repo(pkg)
      graph = "#{cache}/aurlaws/#{pkg}.svg"
      Dir.mkdir_p(File.dirname(graph))
      a = AurHook.audit(dir, graph)
      return STDOUT.puts(a.raw) if @a.json? && !a.raw.empty?
      say AurHook.summary(a.verdict)
      a.verdict.flags.each do |f|
        say "  #{f.severity.ljust(7)} #{f.title}#{f.count > 1 ? " (#{f.count})" : ""}"
      end
      a.verdict.evidence.each { |e| say e }
      say "Graph: #{graph}" if File.exists?(graph)
    end

    private def show_findings(fs : Array(AurGuard::Finding))
      fs.each do |f|
        mark = f.severity.refuse? ? "Stop" : (f.severity.warn? ? "Look" : "Note")
        STDERR.puts "  #{mark}: #{f.text}"
      end
    end

    # Every community package build: pinned commit, TLS metadata, hostile-until-read checks, sandbox, inspection,
    # malware scan, then wold. Never unattended, and -y does not answer these questions.
    def aur_install(pkg : String, update : Bool = false)
      aur.checkout(pkg) || raise Failure.new(6, "#{pkg} is not in the community package collection.")
      dir = aur.repo(pkg)
      raise Failure.new(5, "#{pkg} contains symbolic links, so it was not built.") unless Aur.symlinks(dir).empty?
      commit = AurGuard.head(dir)
      prev = aur.installed_commit(pkg)
      return say("#{pkg} is up to date (#{commit[0, 12]}).") if update && prev == commit
      meta = begin
        aur.meta(pkg)
      rescue e : ArgumentError | IO::Error | OpenSSL::SSL::Error
        raise Failure.new(3, "The community package collection did not answer over a secure connection (#{e.message}), so #{pkg} was not built.")
      end
      log = "#{dir}/.build.log"
      File.delete?(log)
      si = aur.srcinfo(pkg, log)
      pkgbuild = File.read("#{dir}/PKGBUILD")
      inst = Dir.children(dir).select(&.ends_with?(".install")).sort!.map { |f| File.read("#{dir}/#{f}") }.join("\n")
      inst = nil if inst.empty?
      findings = AurGuard.scan(pkgbuild, inst, si) + AurGuard.meta_findings(meta, aur.stored_meta(pkg), Time.utc)
      findings += AurGuard.update_findings(AurGuard.files_at(dir, prev), AurGuard.files_at(dir, commit)) if prev && prev != commit
      STDOUT.puts aur.review_text(pkg, commit)
      note "#{UI.bold(pkg)} from the community package collection, commit #{commit[0, 12]}#{meta.maintainer ? ", maintained by #{meta.maintainer}" : ", with no maintainer"}. Community packages are not reviewed by WolOS: anyone can publish there."
      show_findings(findings)
      note UI.dim(AurGuard::DISCLAIMER)
      skips, hard = findings.select(&.severity.refuse?).partition { |f| f.kind == :skip }
      raise Failure.new(5, "#{pkg} was not built: #{hard.first.text}") unless hard.empty?
      unless skips.empty?
        raise Failure.new(1, "Not built.") unless UI.confirm_strict("A source has no checksum. Build #{pkg} anyway?")
      end
      note Sandbox.describe(true).sub("This step", "Fetching") + " " + Sandbox.describe(false).sub("This step", "The build")
      ok = AurGuard.needs_explicit_yes?(findings) ? UI.confirm_strict("Something changed that needs your decision. Build #{pkg} from what is shown above?") : UI.confirm("Build #{pkg} from what is shown above?", false)
      raise Failure.new(1, "Not built.") unless ok
      raise Failure.new(5, "#{dir} changed after you read it, so nothing was built.") unless AurGuard.pinned?(dir, commit)
      aur.fetch_sources(pkg, log)
      if Scan.needed?("aur")
        note "Checking the downloads for known malware (takes about ten seconds)..."
        v = Scan.run([dir + "/.src"])
        scan_gate(v, "the downloaded sources")
      end
      raise Failure.new(5, "#{dir} changed while it was fetched, so nothing was built.") unless AurGuard.pinned?(dir, commit)
      note "Building #{pkg} in a sandbox as an unprivileged throwaway user..."
      files, trace = aur.compile(pkg, log)
      raise Failure.new(3, "#{pkg} built no package. See #{log}.") if files.empty?
      files.each do |f|
        fs = AurGuard.inspect_file(f)
        show_findings(fs)
        raise Failure.new(5, "#{File.basename(f)} was not installed: #{fs.find!(&.severity.refuse?).text}") if AurGuard.blocked?(fs)
        if sc = AurGuard.scriptlet_of(f)
          STDOUT.puts "==> install script, runs as the administrator\n#{sc}"
          raise Failure.new(1, "Not installed.") unless UI.confirm_strict("Run this install script as the administrator?")
        elsif AurGuard.needs_explicit_yes?(fs)
          raise Failure.new(1, "Not installed.") unless UI.confirm_strict("The built package changes how updates run. Install it?")
        end
        hv = AurHook.check(dir, f, trace)
        hv.evidence.each { |e| STDERR.puts "  #{hv.pass? ? "Note" : "Look"}: #{e}" }
        raise Failure.new(5, "#{File.basename(f)} was not installed: the package analyser blocked it.") if hv.block?
        if hv.flag? && !UI.confirm_strict("The package analyser flagged this build. Install it anyway?")
          raise Failure.new(1, "Not installed.")
        end
      end
      if Scan.needed?("aur")
        note "Checking the built package for known malware..."
        scan_gate(Scan.run(files), "the built package")
      end
      files.each { |f| core.system_call({"call" => "package.install-file", "path" => f}.to_json) }
      aur.write_provenance(pkg, commit, si, meta, files)
      record("aur", update ? "update" : "install", pkg, prev.try(&.[0, 12]), commit[0, 12], {"type" => "run", "argv" => ["wopkg", "remove", pkg]})
      say "Installed #{pkg} (community package, commit #{commit[0, 12]})."
    end

    # One plain sentence for a hit, which stops the install unless the person overrides on purpose;
    # a missing or old signature database says so instead of implying a clean result.
    private def scan_gate(v : Scan::Verdict, what : String)
      if v.infected?
        STDERR.puts v.message
        unless Scan.may_continue?(v, UI.confirm_strict("Install anyway, against the malware check's warning?"))
          raise Failure.new(5, v.message)
        end
      elsif !v.clean?
        note v.message
      end
    end

    def recipe_cmd
      sub = @a.targets[0]? || "list"
      name = @a.targets[1]?
      case sub
      when "list"
        Recipe.dirs.each do |d|
          next unless Dir.exists?(d)
          Dir.children(d).sort.each do |n|
            next unless found = find_recipe(n)
            r = found[0]
            say "#{n.ljust(12)} #{r.version.ljust(10)} #{r.summary}"
          end
        end
      when "build"   then recipe_build(name || raise UsageError.new("wopkg recipe build <name>"))
      when "install" then recipe_install(name || raise UsageError.new("wopkg recipe install <name>"))
      when "remove"  then recipe_remove(name || raise UsageError.new("wopkg recipe remove <name>"))
      when "index"   then STDOUT.print Recipe.index(name || "recipes")
      else                raise UsageError.new("wopkg recipe list|build|install|remove|index")
      end
    end

    # A recipe by name; one that does not read counts as not there (the list skips it).
    private def find_recipe(name : String) : {Recipe, String}?
      Recipe.find(name)
    rescue Recipe::Invalid | File::Error
      nil
    end

    def recipe_build(name : String) : {Recipe, String}
      found = Recipe.find(name) || raise Failure.new(6, "There is no recipe called #{name}.")
      r = found[0]
      b = RecipeBuilder.new(r, cache)
      note(b.cached? ? "#{name} #{r.version}: already built." : "Building #{name} #{r.version} in a sandbox...")
      built = b.run(core, prefix, ENV["WOPKG_VERBOSE"]? == "1") { |step| note "  #{UI.dim(step)}" }
      {r, built}
    end

    def recipe_install(name : String)
      r, built = recipe_build(name)
      files = RecipeBuilder.install(built, prefix)
      dir = "#{home}/.local/share/wopkg/recipes/#{name}"
      Dir.mkdir_p(dir)
      File.write("#{dir}/manifest.json", {"name" => name, "version" => r.version, "files" => files}.to_json)
      record("recipe", "install", name, nil, r.version, {"type" => "run", "argv" => ["wopkg", "recipe", "remove", name]})
      say "Installed #{name} #{r.version} in ~/.local (#{files.size} files)."
    end

    def recipe_remove(name : String)
      m = "#{home}/.local/share/wopkg/recipes/#{name}/manifest.json"
      raise Failure.new(6, "The #{name} recipe is not installed.") unless File.exists?(m)
      j = Manifest.from_json(File.read(m))
      j.files.each { |f| File.delete?(f) if f.starts_with?(prefix + "/") }
      FileUtils.rm_rf(File.dirname(m))
      record("recipe", "remove", name, j.version, nil, {"type" => "run", "argv" => ["wopkg", "recipe", "install", name]})
      say "Removed #{name}."
    end

    def recipe_updates : Array(String)
      base = "#{home}/.local/share/wopkg/recipes"
      return [] of String unless Dir.exists?(base)
      Dir.children(base).select do |n|
        m = begin
          Manifest.from_json(File.read("#{base}/#{n}/manifest.json"))
        rescue JSON::Error | File::Error
          next false
        end
        (r = Recipe.find(n)) && r[0].version != m.version
      end
    end
  end
end
