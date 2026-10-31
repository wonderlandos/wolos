# SPDX-License-Identifier: BSD-2-Clause
module Wopkg
  # Builds (recipes, AUR) run as the person inside bubblewrap: the system read-only, only the build
  # folder writable, a fresh /tmp, no home, no session bus, and no network unless the step is the
  # fetch step. Memory is capped with a systemd user scope when one is available.
  module Sandbox
    MEMORY_MAX = "6G"

    class Refused < Exception
    end

    def self.argv(work : String, cmd : String, net : Bool, chdir : String = "/work/src", env = {} of String => String, memory : Bool = true) : Array(String)
      a = [] of String
      if memory && Process.find_executable("systemd-run")
        a.concat(["systemd-run", "--user", "--scope", "--quiet", "-p", "MemoryMax=#{MEMORY_MAX}", "--"])
      end
      a.concat(%w(bwrap --die-with-parent --new-session --unshare-all --clearenv))
      a << "--share-net" if net
      a.concat(%w(--ro-bind /usr /usr --symlink usr/bin /bin --symlink usr/bin /sbin --symlink usr/lib /lib --symlink usr/lib /lib64))
      a.concat(%w(--ro-bind /etc /etc --proc /proc --dev /dev --tmpfs /tmp --tmpfs /run))
      # resolv.conf usually points into /run: give the fetch step the resolver, nothing else of /run.
      a.concat(%w(--ro-bind-try /run/systemd/resolve /run/systemd/resolve)) if net
      a.concat(["--bind", work, "/work", "--chdir", chdir])
      base = {"PATH" => "/usr/bin", "HOME" => "/work/home", "LANG" => "C.UTF-8", "TMPDIR" => "/tmp",
              "MAKEFLAGS" => "-j4", "CARGO_BUILD_JOBS" => "4", "CARGO_HOME" => "/work/cargo"}
      base.merge(env).each { |k, v| a.concat(["--setenv", k, v]) }
      a.concat(["sh", "-ec", cmd])
      a
    end

    def self.check_user!
      raise Refused.new("wopkg builds as you, never as the administrator. Run it without sudo.") if LibC.getuid == 0
      raise Refused.new("Building needs bubblewrap (bwrap), which is not installed.") unless Process.find_executable("bwrap")
    end

    # Run one step; output goes to `log` and, when `echo`, to the terminal.
    def self.run(work : String, cmd : String, net : Bool, log : String, chdir = "/work/src", env = {} of String => String, echo = false) : Bool
      Dir.mkdir_p("#{work}/home")
      File.open(log, "a") do |f|
        f.puts "$ #{cmd}#{net ? "" : "   (no network)"}"
        f.flush
        sink = echo ? IO::MultiWriter.new(f, STDERR) : f
        a = argv(work, cmd, net, chdir, env)
        Process.run(a.first, a[1..], output: sink, error: sink).success?
      end
    end
  end
end
