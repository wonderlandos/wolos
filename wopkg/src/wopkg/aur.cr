# SPDX-License-Identifier: BSD-2-Clause
require "file_utils"
require "./sandbox"

module Wopkg
  # AUR packages are unreviewed: wopkg shows the PKGBUILD (or what changed since the copy you last
  # accepted), fetches sources in the sandbox with network, builds in the sandbox without network,
  # never as root, and hands the result to wold under the separate `packages-unreviewed` grant.
  class Aur
    BASE = "https://aur.archlinux.org"

    def initialize(@cache : String, @state : String)
    end

    def self.valid?(name : String) : Bool
      name.matches?(/\A[a-z0-9@._+][a-z0-9@._+-]{0,127}\z/)
    end

    def repo(pkg : String) : String
      "#{@cache}/aur/#{pkg}"
    end

    def reviewed(pkg : String) : String
      "#{@state}/aur-reviewed/#{pkg}"
    end

    # Clone or update the AUR git repo (as the person, plain git over HTTPS).
    def checkout(pkg : String) : Bool
      raise ArgumentError.new("“#{pkg}” is not an AUR package name.") unless Aur.valid?(pkg)
      dir = repo(pkg)
      if Dir.exists?("#{dir}/.git")
        Process.run("git", ["-C", dir, "pull", "--ff-only", "-q"], error: STDERR).success?
      else
        Dir.mkdir_p(File.dirname(dir))
        Process.run("git", ["clone", "-q", "--depth", "1", "#{BASE}/#{pkg}.git", dir], error: STDERR).success? &&
          File.exists?("#{dir}/PKGBUILD")
      end
    end

    # Files the review covers: the PKGBUILD and every install script or patch next to it.
    def self.review_files(dir : String) : Array(String)
      Dir.children(dir).reject { |f| f.starts_with?('.') }.select { |f| File.file?("#{dir}/#{f}") && File.size("#{dir}/#{f}") < 1_000_000 }.sort
    end

    # Text to read before building: the whole files the first time, a unified diff later.
    def self.review_text(dir : String, last : String) : String
      if Dir.exists?(last)
        buf = IO::Memory.new
        Process.run("diff", ["-ruN", "--exclude=.git", last, dir], output: buf)
        buf.to_s
      else
        review_files(dir).map { |f| "==> #{f}\n#{File.read("#{dir}/#{f}")}" }.join("\n")
      end
    end

    def accept(pkg : String)
      FileUtils.rm_rf(reviewed(pkg))
      Dir.mkdir_p(reviewed(pkg))
      Aur.review_files(repo(pkg)).each { |f| File.copy("#{repo(pkg)}/#{f}", "#{reviewed(pkg)}/#{f}") }
    end

    # `.SRCINFO` dependency lines.
    def self.depends(srcinfo : String) : Array(String)
      srcinfo.lines.compact_map do |l|
        k, _, v = l.strip.partition(" = ")
        v.split(/[<>=]/).first if %w(depends makedepends checkdepends).includes?(k)
      end.uniq
    end

    # Build in two sandboxed steps; returns the built package files.
    def build(pkg : String, log : String) : Array(String)
      Sandbox.check_user!
      dir = repo(pkg)
      built = "#{dir}/.out"
      FileUtils.rm_rf(built)
      Dir.mkdir_p(built)
      env = {"PKGDEST" => "/work/.out", "SRCDEST" => "/work/.src", "BUILDDIR" => "/work/.build", "PKGEXT" => ".pkg.tar.zst"}
      Sandbox.run(dir, "makepkg --verifysource --nodeps --noconfirm", true, log, "/work", env) ||
        raise ArgumentError.new("The sources of #{pkg} could not be downloaded or did not match. See #{log}.")
      Sandbox.run(dir, "makepkg --nodeps --noconfirm --force --holdver", false, log, "/work", env) ||
        raise ArgumentError.new("#{pkg} did not build. See #{log}.")
      Dir.glob("#{built}/*.pkg.tar.zst").sort
    end
  end
end
