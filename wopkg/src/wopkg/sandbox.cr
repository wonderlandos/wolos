# SPDX-License-Identifier: BSD-2-Clause
module Wopkg
  # Builds (recipes, AUR) run inside bubblewrap: the system read-only, only the build folder
  # writable, a fresh /tmp, no home, no session bus, and no network unless the step is the fetch
  # step. Memory is capped with a systemd user scope when one is available.
  #
  # `strict` is the profile for code nobody reviewed (AUR): a throwaway user namespace where the
  # build is uid 65534 (nobody), not you and not root, with no capabilities and only the few /etc
  # files a build needs. `trace` records the build's system calls for the package analyser.
  module Sandbox
    MEMORY_MAX = "6G"
    # Read-only /etc entries a build needs; nothing else of /etc (no shadow, no ssh, no tokens).
    ETC_TRY = %w(ssl ca-certificates ld.so.cache ld.so.conf ld.so.conf.d localtime os-release passwd group nsswitch.conf hosts pacman.conf pacman.d makepkg.conf.d)

    class Refused < Exception
    end

    def self.describe(net : Bool) : String
      net ? "This step needs the network (it downloads the sources)." : "This step runs with no network."
    end

    def self.quote(s : String) : String
      "'" + s.gsub("'", "'\\''") + "'"
    end

    def self.argv(work : String, cmd : String, net : Bool, chdir : String = "/work/src", env = {} of String => String, memory : Bool = true, strict : Bool = false, trace : String? = nil) : Array(String)
      a = [] of String
      if memory && Process.find_executable("systemd-run")
        a.concat(["systemd-run", "--user", "--scope", "--quiet", "-p", "MemoryMax=#{MEMORY_MAX}", "--"])
      end
      a.concat(%w(bwrap --die-with-parent --new-session --unshare-all --clearenv))
      a.concat(%w(--unshare-user --uid 65534 --gid 65534)) if strict
      a << "--share-net" if net
      a.concat(%w(--ro-bind /usr /usr --symlink usr/bin /bin --symlink usr/bin /sbin --symlink usr/lib /lib --symlink usr/lib /lib64))
      if strict
        a.concat(%w(--ro-bind /etc/makepkg.conf /etc/makepkg.conf))
        ETC_TRY.each { |e| a.concat(["--ro-bind-try", "/etc/#{e}", "/etc/#{e}"]) }
        a.concat(%w(--proc /proc --dev /dev --tmpfs /tmp --tmpfs /run))
        a.concat(%w(--ro-bind-try /etc/resolv.conf /etc/resolv.conf)) if net
      else
        a.concat(%w(--ro-bind /etc /etc --proc /proc --dev /dev --tmpfs /tmp --tmpfs /run))
      end
      # resolv.conf usually points into /run: give the fetch step the resolver, nothing else of /run.
      a.concat(%w(--ro-bind-try /run/systemd/resolve /run/systemd/resolve)) if net
      a.concat(["--bind", work, "/work", "--chdir", chdir])
      a.concat(%w(--cap-drop ALL)) if strict
      base = {"PATH" => "/usr/bin", "HOME" => "/work/home", "LANG" => "C.UTF-8", "TMPDIR" => "/tmp",
              "MAKEFLAGS" => "-j4", "CARGO_BUILD_JOBS" => "4", "CARGO_HOME" => "/work/cargo"}
      base.merge(env).each { |k, v| a.concat(["--setenv", k, v]) }
      body = if t = trace
               "strace -f -qq -o #{t} -e trace=%file,%network,%process sh -ec #{quote(cmd)}"
             else
               cmd
             end
      a.concat(["sh", "-ec", body])
      a
    end

    def self.check_user! : Nil
      raise Refused.new("wopkg builds as you, never as the administrator. Run it without sudo.") if LibC.getuid == 0
      raise Refused.new("Building needs bubblewrap (bwrap), which is not installed.") unless Process.find_executable("bwrap")
    end

    # Run one step; output goes to `log` and, when `echo`, to the terminal.
    def self.run(work : String, cmd : String, net : Bool, log : String, chdir = "/work/src", env = {} of String => String, echo = false, strict : Bool = false, trace : String? = nil) : Bool
      Dir.mkdir_p("#{work}/home")
      File.open(log, "a") do |f|
        f.puts "$ #{cmd}#{net ? "" : "   (no network)"}"
        f.flush
        sink = echo ? IO::MultiWriter.new(f, STDERR) : f
        a = argv(work, cmd, net, chdir, env, strict: strict, trace: trace)
        Process.run(a.first, a[1..], output: sink, error: sink).success?
      end
    end
  end
end