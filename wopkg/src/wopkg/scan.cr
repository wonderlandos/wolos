# SPDX-License-Identifier: GPL-2.0-or-later
module Wopkg
  # The malware scan step (ADR 0021). The engine belongs to the `xguard` stream; until its library
  # lands this runs `clamscan` as a separate process with ClamAV's signature data. ClamAV is GPL, so it is
  # never linked and stays an optional dependency. Repository packages that pacman already verifies
  # are not scanned, so normal installs stay fast.
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

    def self.run(paths : Array(String), clamscan : String? = find_clamscan, db : String = "/var/lib/clamav", now : Time = Time.utc) : Verdict
      unless clamscan
        return Verdict.new(Kind::Unavailable, [] of String, "ClamAV is not installed, so this download was not checked for malware.")
      end
      age = db_age(db, now)
      unless age
        return Verdict.new(Kind::Unavailable, [] of String, "ClamAV has no signature database yet (run freshclam), so this download was not checked for malware.")
      end
      buf = IO::Memory.new
      a = argv(clamscan, paths, db)
      status = Process.run(a.first, a[1..], output: buf, error: Process::Redirect::Close)
      case status.exit_code
      when 0, 1
        hits = buf.to_s.lines.compact_map { |l| l[/: (\S+) FOUND\z/, 1]? }.uniq
        if !hits.empty?
          Verdict.new(Kind::Infected, hits, "ClamAV found #{hits.join(", ")} in the download, so nothing was installed.")
        elsif age > STALE_DAYS
          Verdict.new(Kind::Stale, [] of String, "ClamAV found nothing, but its signatures are #{age} days old, so this is not a clean result. Run freshclam.")
        else
          Verdict.new(Kind::Clean, [] of String, "No known malware found.")
        end
      else
        Verdict.new(Kind::Unavailable, [] of String, "ClamAV could not check this download, so it was not checked for malware.")
      end
    end

    # A hit stops the install unless the person overrides it on purpose.
    def self.may_continue?(v : Verdict, override : Bool) : Bool
      !v.infected? || override
    end
  end
end
