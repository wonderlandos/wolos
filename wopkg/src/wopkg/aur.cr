# SPDX-License-Identifier: BSD-2-Clause
require "file_utils"
require "http/client"
require "./sandbox"
require "./aur_guard"

module Wopkg
  # AUR packages are hostile until read (ADR 0021, AurGuard). wopkg pins the exact commit you
  # reviewed, takes package metadata from the AUR over TLS (not from the search dump), fetches
  # sources in a throwaway sandbox with network, builds in a second one without, and records
  # where every package came from.
  class Aur
    BASE = "https://aur.archlinux.org"
    SAFE = ["-c", "protocol.allow=never", "-c", "protocol.https.allow=always", "-c", "core.hooksPath=/dev/null"]

    def initialize(@cache : String, @state : String)
    end

    def self.valid?(name : String) : Bool
      name.matches?(/\A[a-z0-9@._+][a-z0-9@._+-]{0,127}\z/)
    end

    def repo(pkg : String) : String
      "#{@cache}/aur/#{pkg}"
    end

    def provenance_path(pkg : String) : String
      "#{@state}/aur-provenance/#{pkg}.json"
    end

    def meta_path(pkg : String) : String
      "#{@state}/aur-meta/#{pkg}.json"
    end

    # Packages built through wopkg, with their pinned commit.
    def installed : Array(String)
      Dir.glob("#{@state}/aur-provenance/*.json").map { |f| File.basename(f, ".json") }.sort
    end

    def installed_commit(pkg : String) : String?
      JSON.parse(File.read(provenance_path(pkg)))["commit"]?.try(&.as_s?).try { |c| AurGuard.commit?(c) ? c : nil }
    rescue
      nil
    end

    # The full history is kept so an update can show the diff since the installed commit.
    def checkout(pkg : String) : Bool
      raise ArgumentError.new("“#{pkg}” is not an AUR package name.") unless Aur.valid?(pkg)
      dir = repo(pkg)
      if Dir.exists?("#{dir}/.git")
        Process.run("git", ["-C", dir] + SAFE + ["fetch", "-q", "origin"], error: STDERR).success? &&
          Process.run("git", ["-C", dir] + SAFE + ["reset", "-q", "--hard", "origin/HEAD"], error: STDERR).success?
      else
        Dir.mkdir_p(File.dirname(dir))
        Process.run("git", SAFE + ["clone", "-q", "#{BASE}/#{pkg}.git", dir], error: STDERR).success? &&
          File.exists?("#{dir}/PKGBUILD")
      end
    end

    # Metadata from the AUR itself over TLS; remembered so a later maintainer change shows.
    def meta(pkg : String) : AurGuard::Meta
      r = HTTP::Client.get("#{BASE}/rpc/v5/info?arg[]=#{URI.encode_www_form(pkg)}")
      raise ArgumentError.new("The AUR answered with an error (#{r.status_code}).") unless r.status_code == 200
      AurGuard::Meta.parse(r.body)
    end

    def stored_meta(pkg : String) : AurGuard::Meta?
      j = JSON.parse(File.read(meta_path(pkg)))
      AurGuard::Meta.new(j["maintainer"]?.try(&.as_s?), j["first"].as_i64, j["modified"].as_i64, j["votes"].as_i, 0.0, false)
    rescue
      nil
    end

    def store_meta(pkg : String, m : AurGuard::Meta)
      Dir.mkdir_p(File.dirname(meta_path(pkg)))
      File.write(meta_path(pkg), {"maintainer" => m.maintainer, "first" => m.first_submitted, "modified" => m.last_modified, "votes" => m.votes}.to_json)
    end

    # Symbolic links in a checkout can point a "PKGBUILD" at a file of yours.
    def self.symlinks(dir : String) : Array(String)
      Dir.glob("#{dir}/**/*", match: File::MatchOptions::DotFiles).select { |f| !f.includes?("/.git/") && File.symlink?(f) }.map { |f| f.lchop(dir + "/") }
    end

    def self.review_files(dir : String) : Array(String)
      Dir.children(dir).reject { |f| f.starts_with?('.') }.select { |f| File.file?("#{dir}/#{f}") && !File.symlink?("#{dir}/#{f}") && File.size("#{dir}/#{f}") < 1_000_000 }.sort
    end

    # Text to read before building: the whole files the first time, the diff since the installed commit later.
    def review_text(pkg : String, commit : String) : String
      dir = repo(pkg)
      if (prev = installed_commit(pkg)) && prev != commit
        AurGuard.diff(dir, prev, commit)
      elsif prev == commit
        ""
      else
        Aur.review_files(dir).map { |f| "==> #{f}\n#{File.read("#{dir}/#{f}")}" }.join("\n")
      end
    end

    def self.depends(srcinfo : String) : Array(String)
      srcinfo.lines.compact_map do |l|
        k, _, v = l.strip.partition(" = ")
        v.split(/[<>=]/).first if %w(depends makedepends checkdepends).includes?(k)
      end.uniq
    end

    ENV_BUILD = {"PKGDEST" => "/work/.out", "SRCDEST" => "/work/.src", "BUILDDIR" => "/work/.build", "PKGEXT" => ".pkg.tar.zst"}

    # .SRCINFO as makepkg reads it from the PKGBUILD, inside the sandbox. The one the AUR serves is not used.
    def srcinfo(pkg : String, log : String) : AurGuard::SrcInfo
      dir = repo(pkg)
      File.delete?("#{dir}/.SRCINFO.gen")
      Sandbox.run(dir, "makepkg --printsrcinfo > /work/.SRCINFO.gen", false, log, "/work", ENV_BUILD, strict: true) ||
        raise ArgumentError.new("#{pkg}'s PKGBUILD could not even be read in the sandbox. See #{log}.")
      AurGuard::SrcInfo.parse(File.read("#{dir}/.SRCINFO.gen"))
    end

    # Step 1, with network: download the sources and check checksums and upstream signatures.
    def fetch_sources(pkg : String, log : String)
      Sandbox.check_user!
      dir = repo(pkg)
      FileUtils.rm_rf("#{dir}/.out")
      Dir.mkdir_p("#{dir}/.out")
      Sandbox.run(dir, "makepkg --verifysource --nodeps --noconfirm", true, log, "/work", ENV_BUILD, strict: true) ||
        raise ArgumentError.new("The sources of #{pkg} could not be downloaded or did not match their checksums or signatures. See #{log}.")
    end

    # Step 2, no network: build, recording system calls when strace is there. Returns the packages.
    def compile(pkg : String, log : String) : {Array(String), String?}
      dir = repo(pkg)
      trace = Process.find_executable("strace") ? "/work/.trace" : nil
      File.delete?("#{dir}/.trace")
      Sandbox.run(dir, "makepkg --nodeps --noconfirm --force --holdver", false, log, "/work", ENV_BUILD, strict: true, trace: trace) ||
        raise ArgumentError.new("#{pkg} did not build without network access. It may need to download while building, which wopkg does not allow for AUR packages. See #{log}.")
      {Dir.glob("#{dir}/.out/*.pkg.tar.zst").sort, trace ? "#{dir}/.trace" : nil}
    end

    def write_provenance(pkg : String, commit : String, si : AurGuard::SrcInfo, m : AurGuard::Meta, files : Array(String))
      Dir.mkdir_p(File.dirname(provenance_path(pkg)))
      File.write(provenance_path(pkg), AurGuard.provenance(pkg, commit, si.sources, m.maintainer, files.map { |f| File.basename(f) }, Time.utc))
      store_meta(pkg, m)
    end
  end
end