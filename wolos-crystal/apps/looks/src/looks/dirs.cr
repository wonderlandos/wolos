# SPDX-License-Identifier: GPL-2.0-or-later
# Where Looks reads and writes: its own data folder (state, pictures it was handed, thumbnails), its
# cache (the preview images of the moment), the Pictures and wallpaper folders it offers, and the Night
# Shift flag the desktop keeps. Pure lookups over an environment, so specs pass their own.
module Looks
  APP_ID = "org.wolos.looks"

  module Dirs
    def self.home(env = ENV) : String
      env["HOME"]? || "."
    end

    private def self.absolute(env, key : String) : String?
      (x = env[key]?) && x.starts_with?('/') ? x : nil
    end

    def self.data_home(env = ENV) : String
      absolute(env, "XDG_DATA_HOME") || File.join(home(env), ".local", "share")
    end

    def self.cache_home(env = ENV) : String
      absolute(env, "XDG_CACHE_HOME") || File.join(home(env), ".cache")
    end

    def self.state_home(env = ENV) : String
      absolute(env, "XDG_STATE_HOME") || File.join(home(env), ".local", "state")
    end

    # The app's own data folder: the state file and the pictures the person handed over.
    def self.app_data(env = ENV) : String
      File.join(data_home(env), APP_ID)
    end

    def self.state_file(env = ENV) : String
      File.join(app_data(env), "state.json")
    end

    def self.pictures_copy(env = ENV) : String
      File.join(app_data(env), "pictures")
    end

    def self.cache(env = ENV) : String
      File.join(cache_home(env), APP_ID)
    end

    def self.previews(env = ENV) : String
      File.join(cache(env), "previews")
    end

    def self.thumbnails(env = ENV) : String
      File.join(cache(env), "thumbnails")
    end

    # The person's Pictures folder (as their folder settings name it).
    def self.pictures(env = ENV) : String
      h = home(env)
      settings(env, h) || File.join(h, "Pictures")
    end

    private def self.settings(env, h : String) : String?
      base = absolute(env, "XDG_CONFIG_HOME") || File.join(h, ".config")
      path = File.join(base, "user-dirs.dirs")
      info = File.info?(path)
      return unless info && info.size < 64 * 1024
      File.read(path).each_line do |line|
        if m = line.strip.match(/\AXDG_PICTURES_DIR="(.*)"\z/)
          v = m[1].gsub("$HOME", h)
          return v if v.starts_with?('/') && !v.includes?("..")
        end
      end
      nil
    rescue File::Error
      nil
    end

    # Folders of wallpapers: the shipped ones, then the person's own.
    def self.wallpapers(env = ENV) : Array(String)
      ["/usr/share/wolos/wallpapers", File.join(data_home(env), "wolos", "wallpapers")]
    end

    # The file that exists while Night Shift (Night Light) is on: the desktop's toggle flag
    # (wol_core::toggles::state_file).
    def self.night_shift_flag(env = ENV) : String
      File.join(state_home(env), "wolos", "toggles", "org.wolos.nightlight")
    end

    # The theme the desktop uses now: the first line of the file `wol theme set` writes.
    def self.active_theme_file(env = ENV) : String
      File.join(absolute(env, "XDG_CONFIG_HOME") || File.join(home(env), ".config"), "wolos", "theme")
    end
  end
end
