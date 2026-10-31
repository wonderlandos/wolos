# SPDX-License-Identifier: GPL-2.0-or-later
require "json"
require "uri"

module Wopkg
  # AUR packages are hostile until read (ADR 0021). Everything here is a static check on text the
  # AUR serves: it raises flags in plain words and never says "safe".
  module AurGuard
    DISCLAIMER = "These checks look for known warning signs. They cannot prove a package is safe, and a quiet result is not a guarantee."

    enum Severity
      Note
      Warn
      Refuse
    end

    record Finding, severity : Severity, kind : Symbol, text : String, explicit : Bool = false do
      def explicit? : Bool
        explicit
      end
    end

    SHORTENERS = %w(bit.ly tinyurl.com t.co goo.gl is.gd ow.ly buff.ly rebrand.ly cutt.ly shorturl.at tiny.cc rb.gy)
    PASTES     = %w(pastebin.com paste.ee hastebin.com ghostbin.com 0bin.net termbin.com transfer.sh paste.debian.net gist.githubusercontent.com dpaste.org rentry.co)
    CODE_HOSTS = %w(github.com gitlab.com codeberg.org bitbucket.org sourceforge.net)
    VCS        = %w(git+ svn+ hg+ bzr+ fossil+)
    SUM_ORDER  = %w(b2 sha512 sha384 sha256 sha224 sha1 md5 ck)

    # What `makepkg --printsrcinfo` says (generated inside the sandbox, never taken from the AUR).
    class SrcInfo
      getter url : String
      getter sources : Array({String, String})
      getter keys : Array(String)
      getter? weak : Bool

      def initialize(@url, @sources, @keys, @weak)
      end

      def self.parse(text : String) : SrcInfo
        url = ""
        srcs = [] of String
        sums = Hash(String, Array(String)).new { |h, k| h[k] = [] of String }
        keys = [] of String
        text.each_line do |line|
          k, _, v = line.strip.partition(" = ")
          base = k.sub(/_(x86_64|i686|aarch64|armv7h|armv6h|any)\z/, "")
          case base
          when "url"           then url = v if url.empty?
          when "source"        then srcs << v
          when "validpgpkeys"  then keys << v
          when /\A(b2|sha\d+|md5|ck)sums\z/ then sums[$1] << v
          end
        end
        alg = SUM_ORDER.find { |a| sums.has_key?(a) }
        chosen = alg ? sums[alg] : [] of String
        pairs = srcs.map_with_index { |s, i| {s, chosen[i]? || ""} }
        new(url, pairs, keys, !alg.nil? && %w(sha1 md5 ck).includes?(alg))
      end
    end

    def self.f(sev : Severity, kind : Symbol, text : String, explicit = false) : Finding
      Finding.new(sev, kind, text, explicit)
    end

    def self.host_of(url : String) : String?
      u = url.sub(/\A[\w.-]+::/, "")
      VCS.each { |v| u = u.lchop(v) }
      return nil unless u.includes?("://")
      URI.parse(u.sub(/#.*\z/, "")).host.try(&.downcase)
    rescue
      nil
    end

    def self.scheme_of(url : String) : String?
      u = url.sub(/\A[\w.-]+::/, "")
      VCS.each { |v| u = u.lchop(v) }
      u.includes?("://") ? u.split("://").first.downcase : nil
    end

    def self.registrable(host : String) : String
      host.split('.').last(2).join('.')
    end

    # Is `src` plausibly the project's own host? Same site, or the same owner on a code host.
    def self.related?(page : String, src_url : String) : Bool
      a, b = host_of(page), host_of(src_url)
      return true unless a && b
      return true if a == b || registrable(a) == registrable(b)
      if CODE_HOSTS.includes?(a) && CODE_HOSTS.includes?(b)
        return a == b
      end
      false
    end

    def self.scan_sources(si : SrcInfo, allow_skip : Bool) : Array(Finding)
      acc = [] of Finding
      si.sources.each do |(src, sum)|
        host = host_of(src)
        next unless host
        vcs = VCS.any? { |v| src.sub(/\A[\w.-]+::/, "").starts_with?(v) }
        acc << f(Severity::Warn, :plain_http, "#{host} is reached over plain http, so the download can be swapped on the way.") if scheme_of(src) == "http"
        acc << f(Severity::Warn, :shortener, "A source uses a link shortener (#{host}), which hides where the file really comes from.") if SHORTENERS.includes?(host)
        acc << f(Severity::Warn, :paste, "A source comes from a paste site (#{host}).") if PASTES.includes?(host)
        acc << f(Severity::Warn, :raw_ip, "A source points at a numeric address (#{host}) instead of a named site.") if host.matches?(/\A\d{1,3}(\.\d{1,3}){3}\z/) || host.includes?(':')
        if !si.url.empty? && !related?(si.url, src)
          acc << f(Severity::Warn, :other_host, "A source is downloaded from a different host (#{host}) than the project's own page (#{host_of(si.url)}). Check that it really is the project's repository.")
        end
        if sum == "SKIP" && !vcs
          acc << f(allow_skip ? Severity::Warn : Severity::Refuse, :skip, "The download from #{host} has no checksum (SKIP), so nothing proves it is the file the author meant.")
        end
      end
      acc << f(Severity::Warn, :weak_checksum, "The checksums use md5 or sha1, which can be forged.") if si.weak?
      if !si.keys.empty? && si.sources.none? { |(s, _)| s.matches?(/\.(sig|asc|sign)\z/) }
        acc << f(Severity::Note, :unsigned_keys, "The package lists signing keys, but downloads no signature file, so nothing is actually checked.")
      end
      acc
    end

    WRITE_VERBS = /\b(install|cp|mv|ln|tee|rm|chmod|chown|mkdir|touch|rsync|dd)\b/

    def self.scan_script(text : String, scriptlet : Bool) : Array(Finding)
      acc = [] of Finding
      seen = Set(Symbol).new
      once = ->(k : Symbol, sev : Severity, msg : String) { acc << f(sev, k, msg) if seen.add?(k) }
      text.each_line do |raw|
        line = raw.strip
        next if line.empty? || line.starts_with?('#')
        piped = line.matches?(/\b(curl|wget|fetch)\b[^|\n]*\|\s*(sudo\s+)?(ba|z|da|k)?sh\b/) ||
                line.matches?(/\bbase64\s+(-d|--decode)\b[^|\n]*\|\s*(ba|z|da|k)?sh\b/) ||
                line.matches?(/\b(ba|z|da|k)?sh\s+<\(\s*(curl|wget)/)
        acc << f(Severity::Warn, :pipe_to_shell, "Runs whatever it downloads: `#{line.size > 70 ? line[0, 70] + "..." : line}`.") if piped
        once.call(:eval, Severity::Warn, "Uses eval, which runs text as code and can hide what the script really does.") if line.matches?(/(\A|[;&|(\s])eval\s/)
        once.call(:base64, Severity::Warn, "Decodes base64, a common way to hide what a script does.") if line.matches?(/\bbase64\b/)
        once.call(:sudo, Severity::Warn, "Asks for administrator rights (sudo) while building, which a package never needs to do.") if line.matches?(/(\A|[;&|(\s])(sudo|doas|pkexec)\s/)
        once.call(:home, Severity::Warn, "Reaches into the home folder of whoever builds it.") if line.matches?(/(~|\$\{?HOME\}?)\/\./) || line.matches?(%r{(\A|\s|=|"|')/home/})
        once.call(:systemd, Severity::Warn, "Installs or starts a systemd service, which keeps running after the install.") if line.matches?(%r{systemd/(system|user)/|\.service\b|systemctl})
        if line.matches?(/\bchmod\b\s+(-\w+\s+)?([0-7]?[2467][0-7]{3}|[ugoa]*\+[rwxXt]*s)\b/)
          acc << f(Severity::Warn, :setuid, "Makes a file setuid or setgid, which runs it with extra rights: `#{line.size > 70 ? line[0, 70] + "..." : line}`.")
        end
        unless scriptlet
          toks = line.split(/\s+/).reject { |t| t.includes?("pkgdir") || t.includes?("srcdir") || t.includes?("DESTDIR") || t.includes?("$_") }
          abs = toks.any? { |t| t.gsub(/\A["'>]+|["']+\z/, "").matches?(%r{\A/(usr|etc|opt|bin|sbin|lib|lib64|var|root|boot|srv)(/|\z)}) && !t.starts_with?("--") }
          redirect = line.gsub(/\S*(pkgdir|srcdir|DESTDIR)\S*/, "").matches?(%r{>>?\s*"?/(usr|etc|opt|bin|sbin|lib|var|root|boot)/})
          if (abs && toks.any? { |t| t.matches?(WRITE_VERBS) }) || redirect
            acc << f(Severity::Warn, :outside_pkgdir, "Writes outside the package folder (#{line.size > 60 ? line[0, 60] + "..." : line}), so it would change the build computer itself.")
          end
        end
      end
      acc
    end

    def self.scan(pkgbuild : String, install : String?, si : SrcInfo, allow_skip : Bool = false) : Array(Finding)
      acc = scan_sources(si, allow_skip)
      acc.concat(scan_script(pkgbuild, false))
      if inst = install
        names = inst.scan(/^\s*((?:pre|post)_(?:install|upgrade|remove))\s*\(\)/m).map(&.[1]).uniq
        acc << f(Severity::Warn, :scriptlet, "Has an install script#{names.empty? ? "" : " (#{names.join(", ")})"} that runs as the administrator when the package is installed. Read it first.")
        acc.concat(scan_script(inst, true))
      end
      acc
    end

    # What the AUR RPC says about a package (fetched over TLS; the dump is not used for decisions).
    record Meta, maintainer : String?, first_submitted : Int64, last_modified : Int64, votes : Int32, popularity : Float64, out_of_date : Bool do
      def self.parse(json : String) : Meta
        r = JSON.parse(json)["results"]?.try(&.as_a?).try(&.first?) || raise ArgumentError.new("The AUR did not describe that package.")
        Meta.new(r["Maintainer"]?.try(&.as_s?), r["FirstSubmitted"]?.try(&.as_i64?) || raise(ArgumentError.new("The AUR answer is incomplete.")),
          r["LastModified"]?.try(&.as_i64?) || 0_i64, r["NumVotes"]?.try(&.as_i?) || 0,
          r["Popularity"]?.try(&.as_f?) || 0.0, !r["OutOfDate"]?.try(&.raw).nil?)
      rescue JSON::ParseException
        raise ArgumentError.new("The AUR answer could not be read.")
      end
    end

    def self.meta_findings(m : Meta, prev : Meta?, now : Time) : Array(Finding)
      acc = [] of Finding
      days = ((now.to_unix - m.first_submitted) / 86_400).to_i
      acc << f(Severity::Warn, :new_package, "First submitted to the AUR only #{days} day#{days == 1 ? "" : "s"} ago.") if days < 30
      acc << f(Severity::Note, :few_votes, "Only #{m.votes} vote#{m.votes == 1 ? "" : "s"}, so few people have used it.") if m.votes < 5
      acc << f(Severity::Warn, :orphan, "Has no maintainer, so anyone can take it over.") if m.maintainer.nil?
      if prev && prev.maintainer != m.maintainer
        acc << f(Severity::Warn, :maintainer_changed, "Changed hands: the maintainer was #{prev.maintainer || "nobody"} and is now #{m.maintainer || "nobody"}.", explicit: true)
      end
      acc
    end

    def self.update_findings(old_files : Array(String), new_files : Array(String)) : Array(Finding)
      (new_files - old_files).select(&.ends_with?(".install")).map do |n|
        f(Severity::Warn, :new_scriptlet, "Now has an install script (#{n}) that the version you accepted did not have. It runs as the administrator.", explicit: true)
      end
    end

    def self.needs_explicit_yes?(fs : Array(Finding)) : Bool
      fs.any?(&.explicit?)
    end

    def self.blocked?(fs : Array(Finding)) : Bool
      fs.any? { |x| x.severity.refuse? }
    end

    # ---- git pins -------------------------------------------------------------------------------

    def self.commit?(s : String) : Bool
      s.matches?(/\A[0-9a-f]{40}\z/)
    end

    def self.git(dir : String, args : Array(String)) : {Bool, String}
      buf = IO::Memory.new
      ok = Process.run("git", ["-C", dir, "-c", "core.fsmonitor=false", "-c", "protocol.ext.allow=never"] + args, output: buf, error: Process::Redirect::Close).success?
      {ok, buf.to_s}
    end

    def self.head(dir : String) : String
      ok, acc = git(dir, ["rev-parse", "HEAD"])
      raise ArgumentError.new("#{dir} is not a git checkout.") unless ok && commit?(acc.strip)
      acc.strip
    end

    def self.diff(dir : String, from : String, to : String) : String
      raise ArgumentError.new("Not a commit id.") unless commit?(from) && commit?(to)
      git(dir, ["diff", "--no-color", "--no-ext-diff", from, to, "--", "PKGBUILD", "*.install"])[1]
    end

    def self.files_at(dir : String, commit : String) : Array(String)
      raise ArgumentError.new("Not a commit id.") unless commit?(commit)
      git(dir, ["ls-tree", "--name-only", commit])[1].lines.map(&.strip).reject(&.empty?).sort
    end

    # Is the checkout exactly the reviewed commit, with no edits since?
    def self.pinned?(dir : String, commit : String) : Bool
      return false unless commit?(commit)
      ok, h = git(dir, ["rev-parse", "HEAD"])
      return false unless ok && h.strip == commit
      ok, st = git(dir, ["status", "--porcelain", "--untracked-files=no"])
      ok && st.strip.empty?
    end

    # ---- the built package ---------------------------------------------------------------------

    OK_ROOTS  = %w(usr etc opt var)
    META_FILE = /\A\.(PKGINFO|BUILDINFO|MTREE|INSTALL|CHANGELOG)\z/

    # `bsdtar -tvf pkg.tar.zst` before pacman sees it.
    def self.inspect_listing(listing : String) : Array(Finding)
      acc = [] of Finding
      listing.each_line do |line|
        m = line.match(/\A(\S{10})\s+.*?\s\w{3}\s+\d+\s+(?:\d\d:\d\d|\d{4})\s+(.+)\z/) || next
        mode, path = m[1], m[2].sub(/ (->|link to) .*\z/, "").strip
        first = path.split('/').first
        if mode[0] == 'c' || mode[0] == 'b'
          acc << f(Severity::Refuse, :device, "Contains a device file (#{path}).")
        end
        if mode[3].in?('s', 'S') || mode[6].in?('s', 'S')
          acc << f(Severity::Refuse, :setuid, "#{path} is setuid or setgid, so it would run with extra rights.")
        end
        if path == ".INSTALL"
          acc << f(Severity::Warn, :scriptlet, "Has an install script that runs as the administrator. Read it before installing.")
        elsif path.matches?(META_FILE)
          next
        elsif path.starts_with?('/') || path.split('/').includes?("..") || !OK_ROOTS.includes?(first)
          acc << f(Severity::Refuse, :bad_path, "Puts a file in an unexpected place (#{path}); packages belong under /usr, /etc, /opt or /var.")
        end
        acc << f(Severity::Warn, :hook, "Installs a pacman hook (#{path}), which runs as the administrator on later package changes.", explicit: true) if path.includes?("share/libalpm/hooks/") && !path.ends_with?('/')
        acc << f(Severity::Warn, :systemd, "Installs a systemd unit (#{path}).") if path.includes?("/systemd/") && path.matches?(/\.(service|socket|timer|path)\z/)
      end
      acc
    end

    def self.inspect_file(pkg : String) : Array(Finding)
      buf = IO::Memory.new
      raise ArgumentError.new("The built package could not be listed.") unless Process.run("bsdtar", ["-tvf", pkg], output: buf, error: Process::Redirect::Close).success?
      inspect_listing(buf.to_s)
    end

    # What the install script says (to show it before it can run as the administrator).
    def self.scriptlet_of(pkg : String) : String?
      buf = IO::Memory.new
      ok = Process.run("bsdtar", ["-xOf", pkg, ".INSTALL"], output: buf, error: Process::Redirect::Close).success?
      ok && !buf.to_s.empty? ? buf.to_s : nil
    end

    def self.provenance(pkg : String, commit : String, sources : Array({String, String}), maintainer : String?, files : Array(String), at : Time) : String
      JSON.build do |j|
        j.object do
          j.field "package", pkg
          j.field "commit", commit
          j.field "maintainer", maintainer
          j.field "built", at.to_utc.to_s("%Y-%m-%dT%H:%M:%SZ")
          j.field "sources" do
            j.array { sources.each { |(u, s)| j.object { j.field "url", u; j.field "sha256", s } } }
          end
          j.field "files", files
        end
      end
    end
  end
end
