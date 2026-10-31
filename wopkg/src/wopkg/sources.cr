# SPDX-License-Identifier: BSD-2-Clause
require "json"

module Wopkg
  # Where the index comes from: the repos in pacman.conf, Flatpak's own AppStream copy, the AUR
  # metadata dump, the AppImage catalogue, the `.wol` index and the recipe index.
  module Sources
    ARCH     = "x86_64"
    AUR      = "https://aur.archlinux.org/packages-meta-v1.json.gz"
    APPIMAGE = "https://appimage.github.io/feed.json"
    WOL      = "https://wonderlandos.github.io/wol-packages/index.json"
    RECIPES  = "/usr/share/wopkg/recipes/index.json"

    record Repo, name : String, servers : Array(String)

    # `[repo]` sections (not `[options]`) with their servers, `Include` files read, comments skipped.
    def self.pacman_repos(conf : String, read : Proc(String, String?) = ->(p : String) { File.read(p) rescue nil }) : Array(Repo)
      repos = [] of Repo
      cur : Repo? = nil
      conf.each_line do |raw|
        line = raw.strip
        next if line.empty? || line.starts_with?('#')
        if line.starts_with?('[') && line.ends_with?(']')
          name = line[1..-2]
          cur = name == "options" ? nil : Repo.new(name, [] of String)
          repos << cur.not_nil! if cur
          next
        end
        next unless c = cur
        key, _, val = line.partition('=')
        val = val.strip
        case key.strip
        when "Server"  then c.servers << expand(val, c.name)
        when "Include"
          if text = read.call(val)
            text.each_line do |l|
              k, _, v = l.strip.partition('=')
              c.servers << expand(v.strip, c.name) if k.strip == "Server"
            end
          end
        end
      end
      repos
    end

    def self.expand(url : String, repo : String) : String
      url.gsub("$repo", repo).gsub("$arch", ARCH)
    end

    def self.flatpak_appstream(home : String) : String?
      ["#{home}/.local/share/flatpak/appstream/flathub/#{ARCH}/active/appstream.xml.gz",
       "/var/lib/flatpak/appstream/flathub/#{ARCH}/active/appstream.xml.gz"].find { |p| File.exists?(p) }
    end

    # The index update request for libwopkg (`wopkg_index_update`).
    def self.request(repos : Array(Repo), appstream : String?, local : Bool, only : Array(String)? = nil) : String
      want = ->(k : String) { only.nil? || only.not_nil!.includes?(k) }
      JSON.build do |j|
        j.object do
          j.field "prune", only.nil?
          j.field "sources" do
            j.array do
              if want.call("pacman")
                repos.each do |r|
                  local_db = "/var/lib/pacman/sync/#{r.name}.db"
                  j.object do
                    j.field "name", r.name
                    j.field "kind", "pacman"
                    if (local || r.servers.empty?) && File.exists?(local_db)
                      j.field "path", local_db
                    elsif s = r.servers.find(&.starts_with?("https://"))
                      j.field "url", "#{s}/#{r.name}.db"
                    else
                      j.field "path", local_db
                    end
                  end
                end
              end
              if want.call("flatpak") && (fp = appstream)
                j.object { j.field "name", "flathub"; j.field "kind", "flatpak"; j.field "path", fp }
              end
              j.object { j.field "name", "aur"; j.field "kind", "aur"; j.field "url", AUR } if want.call("aur")
              j.object { j.field "name", "appimagehub"; j.field "kind", "appimage"; j.field "url", APPIMAGE } if want.call("appimage")
              j.object { j.field "name", "wol"; j.field "kind", "wol"; j.field "url", WOL } if want.call("wol")
              if want.call("recipe") && File.exists?(RECIPES)
                j.object { j.field "name", "recipes"; j.field "kind", "recipe"; j.field "path", RECIPES }
              end
            end
          end
        end
      end
    end
  end
end
