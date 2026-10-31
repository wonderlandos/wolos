# SPDX-License-Identifier: GPL-2.0-or-later
module Wopkg
  # Packages the person keeps at their version: one name per line, in wopkg's state folder.
  module Pins
    def self.read(path : String) : Array(String)
      File.exists?(path) ? File.read_lines(path).map(&.strip).reject(&.empty?).uniq : [] of String
    end

    def self.set(path : String, name : String, on : Bool) : Nil
      names = read(path) - [name]
      names << name if on
      Dir.mkdir_p(File.dirname(path))
      File.write(path, names.sort.join("\n") + (names.empty? ? "" : "\n"))
    end
  end
end
