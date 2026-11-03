# SPDX-License-Identifier: BSD-2-Clause
# Where the gallery reads and writes: the person's Pictures folder (as their folder settings name it),
# the app's data folder (thumbnails live there) and the shared Trash folder. Pure lookups over an
# environment, so specs pass their own.
module Gallery
  APP_ID = "org.wolos.gallery"

  module Dirs
    # The folder settings file names the Pictures folder like `XDG_PICTURES_DIR="$HOME/Photos"`.
    def self.pictures(env = ENV, config : String? = nil) : String
      home = env["HOME"]? || "."
      text = config || read_settings(env, home)
      if text
        text.each_line do |line|
          if m = line.strip.match(/\AXDG_PICTURES_DIR="(.*)"\z/)
            value = m[1].gsub("$HOME", home)
            return value if value.starts_with?('/') && !value.includes?("..")
          end
        end
      end
      File.join(home, "Pictures")
    end

    # The data folder of the person (where apps keep what is theirs).
    def self.data_home(env = ENV) : String
      if (x = env["XDG_DATA_HOME"]?) && x.starts_with?('/')
        x
      else
        File.join(env["HOME"]? || ".", ".local", "share")
      end
    end

    # This app's own data folder.
    def self.app_data(env = ENV) : String
      File.join(data_home(env), APP_ID)
    end

    def self.thumbnails(env = ENV) : String
      File.join(app_data(env), "thumbnails")
    end

    # The Trash every file manager of the session shows.
    def self.trash(env = ENV) : String
      File.join(data_home(env), "Trash")
    end

    private def self.read_settings(env, home : String) : String?
      base = (x = env["XDG_CONFIG_HOME"]?) && x.starts_with?('/') ? x : File.join(home, ".config")
      path = File.join(base, "user-dirs.dirs")
      File.info?(path).try { |i| i.size < 64 * 1024 ? File.read(path) : nil }
    rescue File::Error
      nil
    end
  end
end
