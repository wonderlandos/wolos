# SPDX-License-Identifier: GPL-2.0-or-later
# The real-sandbox specs need bwrap and user namespaces, which a chroot or a locked-down build
# container does not have; there they say so and pass over the run. Sandbox.run also puts the
# build in a user scope (systemd-run --user --scope, the memory cap) wherever systemd-run exists,
# so a clean chroot with systemd-run but no user manager is not a place to run them either.
def sandbox_runs? : Bool
  return false unless Process.find_executable("bwrap")
  unless Process.run("bwrap", ["--unshare-all", "--ro-bind", "/", "/", "true"]).success?
    STDERR.puts "no user namespaces here: real-sandbox specs not run"
    return false
  end
  if Process.find_executable("systemd-run") &&
     !Process.run("systemd-run", ["--user", "--scope", "--quiet", "true"]).success?
    STDERR.puts "no user service manager here (systemd-run --user --scope fails): real-sandbox specs not run"
    return false
  end
  true
end
