# SPDX-License-Identifier: BSD-2-Clause
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

module Wopkg
  # The commands. Every system change is one wold call through libwopkg; everything else runs as
  # the person. `--json` prints libwopkg's documents unchanged.
  class App
    getter core : Core
    @a : Args

    def initialize(@a : Args, config = ENV["WOPKG_CONFIG"]? || "{}")
      @config = JSON.parse(config).as_h? || {} of String => JSON::Any
      @core = Core.new(config)
    end

    def home : String
      @config["home"]?.try(&.as_s) || ENV["HOME"]
    end

    def cache : String
      @config["cache_dir"]?.try(&.as_s) || "#{ENV["XDG_CACHE_HOME"]? || "#{home}/.cache"}/wopkg"
    end

    def state : String
      @config["state_dir"]?.try(&.as_s) || "#{ENV["XDG_STATE_HOME"]? || "#{home}/.local/state"}/wopkg"
    end

    def prefix : String
      "#{home}/.local"
    end

    def say(s : String)
      STDOUT.puts s unless @a.json
    end

    def note(s : String)
      STDERR.puts s
    end

    def emit(j)
      STDOUT.puts j.to_json if @a.json
    end

    def run : Int32
      case @a.verb
      when "help"        then help
      when "version"     then say "wopkg #{VERSION} (libwopkg #{Core.version})"
      when "__fetch"     then bench_fetch
      when "__complete"  then STDOUT.puts core.complete(@a.targets.first? || "") rescue nil
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
      when "rollback"    then rollback(@a.targets.first)
      when "channel"     then channel
      when "pause"       then pause
      when "resume"      then File.delete?("#{state}/pause"); say "Automatic system updates are back on."
      when "appimage"    then appimage
      when "aur"         then aur_cmd
      when "audit"       then audit
      when "recipe"      then recipe_cmd
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

          wopkg search <words>      (-Ss)  find apps in WolOS, Arch, Flathub, AppImage, recipes, AUR
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
          wopkg channel [stable|beta|nightly] | pause [days] | resume
          wopkg appimage add <url|file> | wopkg aur review|update <pkg> | wopkg audit <pkg> | wopkg recipe build|install|remove <name>
          wopkg completions zsh|bash|fish

        Options: -y (do not ask), --json (for tools).
        H
    end

    def completions
      shell = @a.targets.first? || "zsh"
      s = Completions.for(shell) || raise UsageError.new("Completions exist for zsh, bash and fish.")
      STDOUT.print s
    end

    def repos : Array(Sources::Repo)
      conf = ENV["WOPKG_PACMAN_CONF"]? || "/etc/pacman.conf"
      File.exists?(conf) ? Sources.pacman_repos(File.read(conf)) : [] of Sources::Repo
    end

    def index_request : String
      Sources.request(repos, Sources.flatpak_appstream(home), @a.local, @a.only)
    end

    def sync
      t = Time.instant
      res = core.index_update(index_request, UI.progress("index"))
      return emit(res) if @a.json
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
      dir = @config["cache_dir"]?.try(&.as_s) || cache
      return if Dir.exists?("#{dir}/index") && !Dir.children("#{dir}/index").empty?
      note "Building the search index for the first time..."
      core.index_update(index_request, UI.progress("index"))
    end

    def search
      ensure_index
      res = core.search(@a.targets.join(" "), 30)
      return emit(res) if @a.json
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
      return emit(res) if @a.json
      say UI.bold(res["name"].as_s)
      say "  #{res["summary"].as_s}"
      res["sources"].as_a.each do |s|
        say "  #{UI.source_label(s).ljust(28)} #{s["version"].as_s.ljust(18)} #{s["id"].as_s}#{s["installed"].as_bool ? "  [installed]" : ""}"
      end
    end

    def list
      res = core.installed
      return emit(res) if @a.json
      {"pacman" => "System packages", "flatpak" => "Flatpak", "wol" => "WolOS apps", "recipe" => "Recipes"}.each do |k, title|
        v = res[k].as_a
        say "#{title} (#{v.size}): #{v.first(12).map(&.as_s).join(", ")}#{v.size > 12 ? ", ..." : ""}" unless v.empty?
      end
      a = res["appimage"].as_a
      say "AppImages (#{a.size}): #{a.map(&.["name"].as_s).join(", ")}" unless a.empty?
    end

    def sources
      JSON.parse(index_request)["sources"].as_a.each do |s|
        say "#{s["name"].as_s.ljust(16)} #{s["kind"].as_s.ljust(9)} #{(s["url"]? || s["path"]).not_nil!.as_s}"
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
    rescue
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
        note "The AUR is not reviewed by WolOS: anyone can publish there. Read what it does before you build it."
      end
      return say "Not installed." unless UI.confirm("Install it from #{UI.source_label(s)}?", @a.yes && kind != "aur")
      case kind
      when "pacman"
        core.system_call({"call" => "package.change", "verb" => "install", "names" => [id]}.to_json)
      when "flatpak"
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
      r = HTTP::Client.get("https://api.github.com/repos/#{repo}/releases/latest", headers: HTTP::Headers{"Accept" => "application/vnd.github+json", "User-Agent" => "wopkg"})
      raise Failure.new(3, "GitHub has no release for #{repo}.") unless r.status_code == 200
      assets = JSON.parse(r.body)["assets"].as_a.select { |x| x["name"].as_s.downcase.ends_with?(".appimage") }
      a = assets.find { |x| x["name"].as_s.includes?("x86_64") || x["name"].as_s.includes?("amd64") } || assets.first? ||
          raise Failure.new(6, "The latest #{repo} release has no AppImage.")
      a["browser_download_url"].as_s
    end

    def remove(name : String)
      inst = core.installed
      if inst["appimage"].as_a.any? { |x| x["name"].as_s == name }
        return say "Not removed." unless UI.confirm("Remove #{name} (AppImage)?", @a.yes)
        id = core.appimage_remove(name)
        return say "Removed #{name}. Undo: wopkg rollback #{id}"
      end
      hit = (core.info(name) rescue nil)
      srcs = hit ? hit["sources"].as_a.select(&.["installed"].as_bool) : [] of JSON::Any
      s = srcs.find { |x| @a.from.nil? || x["kind"].as_s == @a.from } || raise Failure.new(6, "#{name} is not installed.")
      return say "Not removed." unless UI.confirm("Remove #{hit.not_nil!["name"].as_s} (#{UI.source_label(s)})?", @a.yes)
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
      say "Removed #{hit.not_nil!["name"].as_s}."
    end

    def want?(kind : String) : Bool
      @a.only.nil? || @a.only.not_nil!.includes?(kind)
    end

    # One command for everything: WolOS release, system packages, Flatpaks, AppImages, .wol, recipes.
    def update
      summary = [] of String
      (core.index_update(index_request) rescue nil) if @a.only.nil? || want?("pacman")
      if want?("system")
        plan = core.release_plan(({"to" => @a.to}.compact).to_json) rescue nil
        if plan
          case plan["reason"].as_s
          when "available"
            t = plan["target"]
            summary << "WolOS #{t["version"].as_s} (from #{plan["current"].as_s})"
            if !@a.check && UI.confirm("Update WolOS to #{t["version"].as_s}? It installs next to the running system and starts on restart.", @a.yes)
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
        if @a.check
          buf = IO::Memory.new
          Process.run("pacman", ["-Qu"], output: buf) rescue nil
          n = buf.to_s.lines.size
          summary << "#{n} system package#{n == 1 ? "" : "s"}" if n > 0
        else
          begin
            core.system_call({"call" => "package.change", "verb" => "update", "names" => [] of String}.to_json)
            summary << "system packages"
          rescue e : Failure
            note e.message.to_s unless e.code == 3 && @a.only.nil?
          end
        end
      end
      if want?("flatpak") && @a.to.nil? && (ids = Flatpak.capture(Flatpak::UPDATES_ARGV).try { |o| Flatpak.ids(o) }) && !ids.empty?
        summary << "#{ids.size} Flatpak app#{ids.size == 1 ? "" : "s"}"
        unless @a.check
          commits = ids.to_h { |i| {i, Flatpak.capture(Flatpak.commit_argv(i)).try(&.strip)} }
          if Flatpak.run(Flatpak.update_argv)
            commits.each { |i, c| record("flatpak", "update", i, c, nil, c ? {"type" => "run", "argv" => Flatpak.rollback_argv(i, c)} : nil) }
          end
        end
      end
      if want?("appimage") && @a.to.nil?
        r = core.appimage_update({"check_only" => @a.check, "adopt" => true}.to_json, UI.progress("AppImage"))
        r["apps"].as_a.each do |x|
          case x["state"].as_s
          when "updated"   then summary << "#{x["name"].as_s} (AppImage, #{UI.bytes(x["fetched_bytes"].as_i64)} downloaded)"
          when "available" then summary << "#{x["name"].as_s} (AppImage)"
          when "failed"    then note "#{x["name"].as_s}: #{x["message"].as_s}"
          end
        end
      end
      if want?("wol") && @a.to.nil? && Process.find_executable("wol")
        Process.run("wol", ["update", "apps"] + (@a.check ? [] of String : ["--yes"]), output: STDERR, error: STDERR)
      end
      if want?("recipe") && @a.to.nil?
        recipe_updates.each do |n|
          summary << "#{n} (recipe)"
          recipe_install(n) unless @a.check
        end
      end
      if (n = aur.installed.size) > 0 && !@a.json && (@a.only.nil? || want?("aur"))
        note "#{n} AUR package#{n == 1 ? "" : "s"} built here: AUR updates never run on their own. Read and update them with `wopkg aur update`."
      end
      if @a.json
        emit({"changes" => summary, "checked_only" => @a.check})
      elsif summary.empty?
        say "Everything is up to date."
      else
        say "#{@a.check ? "Updates available" : "Updated"}: #{summary.join(", ")}."
      end
    end

    def history
      h = core.history
      return emit(h) if @a.json
      h["entries"].as_a.last(30).each do |e|
        t = Time.unix(e["time"].as_i64).to_local.to_s("%Y-%m-%d %H:%M")
        undone = e["undone"].as_bool ? "  (undone)" : ""
        ver = [e["from"]?.try(&.as_s?), e["to"]?.try(&.as_s?)].compact.map { |v| v.size > 16 ? v[0, 12] : v }.join(" -> ")
        say "#{e["id"].as_s.rjust(4)}  #{t}  #{e["action"].as_s.ljust(8)} #{e["source"].as_s.ljust(8)} #{e["name"].as_s} #{ver}#{undone}"
      end
    end

    def rollback(id : String)
      if id == "system"
        return say "Nothing changed." unless UI.confirm("Start the previous WolOS on the next restart?", @a.yes)
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

    def channel
      if c = @a.targets.first?
        core.system_call({"call" => "system.channel", "channel" => c}.to_json)
        say "This computer now follows the #{c} channel."
      else
        say (File.read("/etc/wolos/channel").strip rescue "stable")
      end
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
        aur.checkout(pkg) || raise Failure.new(6, "#{pkg} is not in the AUR.")
        text = aur.review_text(pkg, AurGuard.head(aur.repo(pkg)))
        STDOUT.puts(text.empty? ? "Nothing changed since the version you installed." : text)
      when "update"
        (pkg ? [pkg] : aur.installed).each { |n| aur_install(n, true) }
        say "No AUR packages built by wopkg yet." if pkg.nil? && aur.installed.empty?
      else
        raise UsageError.new("wopkg aur review <package> | wopkg aur update [package]")
      end
    end

    # What an AUR package's files say, read and never run. Nothing here builds or installs.
    def audit
      pkg = @a.targets.first? || raise UsageError.new("wopkg audit <package>")
      raise UsageError.new("#{pkg} is not a package name.") unless Aur.valid?(pkg)
      aur.checkout(pkg) || raise Failure.new(6, "#{pkg} is not in the AUR.")
      dir = aur.repo(pkg)
      graph = "#{cache}/aurlaws/#{pkg}.svg"
      Dir.mkdir_p(File.dirname(graph))
      a = AurHook.audit(dir, graph)
      return STDOUT.puts(a.raw) if @a.json && !a.raw.empty?
      a.verdict.evidence.each { |e| say e }
      say AurHook.summary(a.verdict)
      say "Graph: #{graph}" if File.exists?(graph)
    end

    private def show_findings(fs : Array(AurGuard::Finding))
      fs.each do |f|
        mark = f.severity.refuse? ? "Stop" : (f.severity.warn? ? "Look" : "Note")
        STDERR.puts "  #{mark}: #{f.text}"
      end
    end

    # Every AUR build: pinned commit, TLS metadata, hostile-until-read checks, sandbox, inspection,
    # malware scan, then wold. Never unattended, and -y does not answer these questions.
    def aur_install(pkg : String, update : Bool = false)
      aur.checkout(pkg) || raise Failure.new(6, "#{pkg} is not in the AUR.")
      dir = aur.repo(pkg)
      raise Failure.new(5, "#{pkg} contains symbolic links, so it was not built.") unless Aur.symlinks(dir).empty?
      commit = AurGuard.head(dir)
      prev = aur.installed_commit(pkg)
      return say("#{pkg} is up to date (#{commit[0, 12]}).") if update && prev == commit
      meta = begin
        aur.meta(pkg)
      rescue e
        raise Failure.new(3, "The AUR did not answer over a secure connection (#{e.message}), so #{pkg} was not built.")
      end
      log = "#{dir}/.build.log"
      File.delete?(log)
      si = aur.srcinfo(pkg, log)
      pkgbuild = File.read("#{dir}/PKGBUILD")
      inst = Dir.children(dir).select(&.ends_with?(".install")).sort.map { |f| File.read("#{dir}/#{f}") }.join("\n")
      inst = nil if inst.empty?
      findings = AurGuard.scan(pkgbuild, inst, si) + AurGuard.meta_findings(meta, aur.stored_meta(pkg), Time.utc)
      findings += AurGuard.update_findings(AurGuard.files_at(dir, prev), AurGuard.files_at(dir, commit)) if prev && prev != commit
      STDOUT.puts aur.review_text(pkg, commit)
      note "#{UI.bold(pkg)} from the AUR, commit #{commit[0, 12]}#{meta.maintainer ? ", maintained by #{meta.maintainer}" : ", with no maintainer"}. The AUR is not reviewed by WolOS: anyone can publish there."
      show_findings(findings)
      note UI.dim(AurGuard::DISCLAIMER)
      skips, hard = findings.select { |f| f.severity.refuse? }.partition { |f| f.kind == :skip }
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
        raise Failure.new(5, "#{File.basename(f)} was not installed: #{fs.find(&.severity.refuse?).not_nil!.text}") if AurGuard.blocked?(fs)
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
      say "Installed #{pkg} (AUR #{commit[0, 12]})."
    end

    # One plain sentence for a hit, which stops the install unless the person overrides on purpose;
    # a missing or old signature database says so instead of implying a clean result.
    private def scan_gate(v : Scan::Verdict, what : String)
      if v.infected?
        STDERR.puts v.message
        unless Scan.may_continue?(v, UI.confirm_strict("Install anyway, against ClamAV's warning?"))
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
            r = Recipe.find(n) rescue nil
            say "#{n.ljust(12)} #{r.version.ljust(10)} #{r.summary}" if r = r.try(&.[0])
          end
        end
      when "build"   then recipe_build(name || raise UsageError.new("wopkg recipe build <name>"))
      when "install" then recipe_install(name || raise UsageError.new("wopkg recipe install <name>"))
      when "remove"  then recipe_remove(name || raise UsageError.new("wopkg recipe remove <name>"))
      when "index"   then STDOUT.print Recipe.index(name || "recipes")
      else                raise UsageError.new("wopkg recipe list|build|install|remove|index")
      end
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
      j = JSON.parse(File.read(m))
      j["files"].as_a.each { |f| File.delete?(f.as_s) if f.as_s.starts_with?(prefix + "/") }
      FileUtils.rm_rf(File.dirname(m))
      record("recipe", "remove", name, j["version"].as_s, nil, {"type" => "run", "argv" => ["wopkg", "recipe", "install", name]})
      say "Removed #{name}."
    end

    def recipe_updates : Array(String)
      base = "#{home}/.local/share/wopkg/recipes"
      return [] of String unless Dir.exists?(base)
      Dir.children(base).select do |n|
        m = JSON.parse(File.read("#{base}/#{n}/manifest.json")) rescue next false
        (r = Recipe.find(n)) && r[0].version != m["version"].as_s
      end
    end
  end
end
