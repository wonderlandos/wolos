# SPDX-License-Identifier: BSD-2-Clause
require "json"

module Wopkg
  # Human output: one line per thing, a progress bar on a terminal, one confirmation question.
  module UI
    SOURCE_NAMES = {"pacman" => "WolOS", "flatpak" => "Flathub", "aur" => "AUR (unreviewed)",
                    "appimage" => "AppImage", "recipe" => "recipe", "wol" => "WolOS app"}

    def self.tty? : Bool
      STDERR.tty?
    end

    def self.bold(s : String) : String
      tty? && !ENV.has_key?("NO_COLOR") ? "\e[1m#{s}\e[0m" : s
    end

    def self.dim(s : String) : String
      tty? && !ENV.has_key?("NO_COLOR") ? "\e[2m#{s}\e[0m" : s
    end

    # "Arch (extra)", "WolOS", "Flathub", "AUR (unreviewed)".
    def self.source_label(s : JSON::Any) : String
      kind = s["kind"].as_s
      repo = s["repo"].as_s
      return "WolOS" if kind == "pacman" && repo == "wolos"
      return "WolOS optimized (#{repo})" if kind == "pacman" && (repo.starts_with?("wolos-cachy") || repo.starts_with?("cachyos"))
      return "Arch (#{repo})" if kind == "pacman"
      SOURCE_NAMES[kind]? || kind
    end

    def self.bytes(n : Int64 | Int32 | Float64) : String
      f = n.to_f
      return "#{f.to_i} B" if f < 1024
      return "%.1f KiB" % (f / 1024) if f < 1024 * 1024
      return "%.1f MiB" % (f / 1024 / 1024) if f < 1024.0 ** 3
      "%.2f GiB" % (f / 1024 ** 3)
    end

    def self.bar(done : Int64, total : Int64?, width = 24) : String
      return "#{bytes(done)}" unless total && total > 0
      frac = (done.to_f / total).clamp(0.0, 1.0)
      fill = (frac * width).round.to_i
      "[#{"#" * fill}#{"." * (width - fill)}] #{(frac * 100).round.to_i}% #{bytes(done)}/#{bytes(total)}"
    end

    # A progress sink for libwopkg events (bytes, done, source, appimage).
    def self.progress(label : String) : Proc(JSON::Any, Nil)
      ->(e : JSON::Any) {
        return unless tty?
        case e["event"]?.try(&.as_s)
        when "bytes"
          total = e["total"]?.try(&.as_i64?)
          STDERR.print "\r\e[K  #{label}  #{bar(e["done"].as_i64, total)}"
        when "done"
          STDERR.print "\r\e[K"
        end
        nil
      }
    end

    def self.search_line(h : JSON::Any) : String
      srcs = h["sources"].as_a
      top = srcs.first
      ver = top["version"].as_s
      inst = srcs.any?(&.["installed"].as_bool) ? " #{bold("[installed]")}" : ""
      head = "#{bold(h["name"].as_s)} #{ver}#{inst}"
      where = srcs.map { |s| label = source_label(s); s["kind"].as_s == "flatpak" ? "#{label} #{s["id"].as_s}" : label }.uniq.join(" · ")
      "#{head}\n    #{h["summary"].as_s}\n    #{dim(where)}"
    end

    # "Install firefox 140.0-1 from Arch (extra)? [Y/n]" -> true for yes (Enter is yes).
    # For risky choices: only the word "yes" counts. -y never answers it, Enter means no.
    def self.confirm_strict(q : String, input = STDIN) : Bool
      STDERR.print "#{q} Type yes to go on: "
      input.gets.try(&.strip.downcase) == "yes"
    end

    def self.confirm(q : String, yes : Bool, input = STDIN) : Bool
      return true if yes
      STDERR.print "#{q} [Y/n] "
      a = input.gets.try(&.strip.downcase) || "n"
      a.empty? || a == "y" || a == "yes"
    end
  end
end
