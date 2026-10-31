# SPDX-License-Identifier: BSD-2-Clause
module Wopkg
  # Flatpak apps for the person (`--user`): no privilege. Flatpak checks GPG on every pull; wopkg
  # refuses a remote that has it turned off.
  module Flatpak
    def self.install_argv(remote : String, id : String) : Array(String)
      ["flatpak", "install", "--user", "--noninteractive", "-y", remote, id]
    end

    def self.remove_argv(id : String) : Array(String)
      ["flatpak", "uninstall", "--user", "--noninteractive", "-y", id]
    end

    def self.update_argv(ids : Array(String) = [] of String) : Array(String)
      ["flatpak", "update", "--user", "--noninteractive", "-y"] + ids
    end

    UPDATES_ARGV = ["flatpak", "remote-ls", "--user", "--updates", "--app", "--columns=application"]
    REMOTES_ARGV = ["flatpak", "remotes", "--user", "--show-disabled", "--columns=name,options"]

    def self.commit_argv(id : String) : Array(String)
      ["flatpak", "info", "--user", "--show-commit", id]
    end

    # Undo for an update: back to the commit it had.
    def self.rollback_argv(id : String, commit : String) : Array(String)
      ["flatpak", "update", "--user", "--noninteractive", "-y", "--commit=#{commit}", id]
    end

    # Remote names whose options include `no-gpg-verify`.
    def self.unsigned_remotes(remotes_output : String) : Array(String)
      remotes_output.lines.compact_map do |l|
        f = l.split(/\s+/, 2)
        f[0] if f.size == 2 && f[1].split(',').map(&.strip).includes?("no-gpg-verify")
      end
    end

    def self.ids(output : String) : Array(String)
      output.lines.map(&.strip).reject { |l| l.empty? || l == "Application ID" }
    end

    def self.capture(argv : Array(String)) : String?
      return nil unless Process.find_executable(argv[0])
      buf = IO::Memory.new
      st = Process.run(argv[0], argv[1..], output: buf, error: Process::Redirect::Close)
      st.success? ? buf.to_s : nil
    end

    def self.run(argv : Array(String)) : Bool
      Process.find_executable(argv[0]) ? Process.run(argv[0], argv[1..], output: STDERR, error: STDERR).success? : false
    end
  end
end
