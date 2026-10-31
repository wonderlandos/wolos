# SPDX-License-Identifier: GPL-2.0-or-later
# The real-sandbox specs need bwrap and user namespaces, which a chroot or a locked-down build
# container does not have; there they say so and pass over the run.
def sandbox_runs? : Bool
  return false unless Process.find_executable("bwrap")
  return true if Process.run("bwrap", ["--unshare-all", "--ro-bind", "/", "/", "true"]).success?
  STDERR.puts "no user namespaces here: real-sandbox specs not run"
  false
end
