# SPDX-License-Identifier: GPL-2.0-or-later
require "json"

module Wopkg
  # The malware scan step (ADR 0021, 0026). `wol-xguard scan --json` reads archives in memory and asks
  # the full signature engine in its own process when the person turned the full signatures on; `clamscan` is the
  # fallback off WolOS. Neither is linked. Repository packages that pacman already verifies are not
  # scanned, so normal installs stay fast.
  module Scan
    enum Kind
      Clean
      Infected
      Unavailable
      Stale
    end

    STALE_DAYS = 7
    DB_FILES   = %w(daily.cvd daily.cld main.cvd main.cld bytecode.cvd bytecode.cld)

    record Verdict, kind : Kind, hits : Array(String), message : String do
      def clean? : Bool
        kind.clean?
      end

      def infected? : Bool
        kind.infected?
      end

      def unavailable? : Bool
        kind.unavailable?
      end

      def stale? : Bool
        kind.stale?
      end
    end

    # Does this install need a scan? Signed repositories (pacman, Flatpak remotes) do not.
    def self.needed?(kind : String, third_party : Bool = true) : Bool
      case kind
      when "aur", "appimage", "recipe" then true
      when "wol"                       then third_party
      else                                  false
      end
    end

    def self.argv(clamscan : String, paths : Array(String), db : String) : Array(String)
      [clamscan, "--database=#{db}", "--no-summary", "--recursive", "--"] + paths
    end

    # Age in days of the newest signature file, or nil when there is none.
    def self.db_age(db : String, now : Time) : Int32?
      newest = DB_FILES.compact_map { |f| File.info?("#{db}/#{f}").try(&.modification_time) }.max?
      newest ? (now - newest).total_days.floor.to_i : nil
    end

    def self.find_clamscan : String?
      Process.find_executable("clamscan")
    end

    def self.find_xguard : String?
      Process.find_executable("wol-xguard")
    end

    # One line of `wol-xguard scan --json`; what it adds beyond these fields is ignored.
    struct XguardReport
      include JSON::Serializable

      struct Hit
        include JSON::Serializable

        getter name : String
      end

      getter findings : Array(Hit) = [] of Hit
      getter signatures_age_days : Int64? = nil
    end

    # One report per line, exit 0 clean, 1 found, 2 incomplete, 3 could not run.
    def self.run_xguard(paths : Array(String), xguard : String, now : Time = Time.utc) : Verdict
      hits = [] of String
      age = nil.as(Int64?)
      incomplete = false
      paths.each do |p|
        buf = IO::Memory.new
        status = Process.run(xguard, ["scan", "--json", p], output: buf, error: Process::Redirect::Close)
        return Verdict.new(Kind::Unavailable, [] of String, "The malware check could not run, so this download was not checked.") if status.exit_code == 3
        r = begin
          XguardReport.from_json(buf.to_s.lines.first? || "{}")
        rescue JSON::Error
          return Verdict.new(Kind::Unavailable, [] of String, "The malware check gave an answer that could not be read, so this download was not checked.")
        end
        r.findings.each { |f| hits << f.name }
        age = r.signatures_age_days || age
        incomplete ||= status.exit_code == 2
      end
      hits = hits.uniq
      if !hits.empty?
        Verdict.new(Kind::Infected, hits, "The malware check found #{hits.join(", ")} in the download, so nothing was installed.")
      elsif incomplete
        Verdict.new(Kind::Unavailable, [] of String, "Part of this download could not be checked for malware.")
      elsif (a = age) && a > STALE_DAYS
        Verdict.new(Kind::Stale, [] of String, "Nothing was found, but the signatures are #{a} days old, so this is not a clean result.")
      else
        Verdict.new(Kind::Clean, [] of String, "No known malware found.")
      end
    end

    def self.run(paths : Array(String), clamscan : String? = find_clamscan, db : String = "/var/lib/clamav", now : Time = Time.utc, xguard : String? = find_xguard) : Verdict
      if xguard
        return run_xguard(paths, xguard, now)
      end
      unless clamscan
        return Verdict.new(Kind::Unavailable, [] of String, "The full signature engine is not installed, so this download was not checked for malware.")
      end
      age = db_age(db, now)
      unless age
        return Verdict.new(Kind::Unavailable, [] of String, "The full signature engine has no signature database yet (run freshclam), so this download was not checked for malware.")
      end
      buf = IO::Memory.new
      a = argv(clamscan, paths, db)
      status = Process.run(a.first, a[1..], output: buf, error: Process::Redirect::Close)
      case status.exit_code
      when 0, 1
        hits = buf.to_s.lines.compact_map { |l| l[/: (\S+) FOUND\z/, 1]? }.uniq!
        if !hits.empty?
          Verdict.new(Kind::Infected, hits, "The full signature engine found #{hits.join(", ")} in the download, so nothing was installed.")
        elsif age > STALE_DAYS
          Verdict.new(Kind::Stale, [] of String, "The full signature engine found nothing, but its signatures are #{age} days old, so this is not a clean result. Run freshclam.")
        else
          Verdict.new(Kind::Clean, [] of String, "No known malware found.")
        end
      else
        Verdict.new(Kind::Unavailable, [] of String, "The full signature engine could not check this download, so it was not checked for malware.")
      end
    end

    # A hit stops the install unless the person overrides it on purpose.
    def self.may_continue?(v : Verdict, override : Bool) : Bool
      !v.infected? || override
    end
  end
end
