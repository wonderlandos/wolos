# SPDX-License-Identifier: GPL-2.0-or-later
require "spec"
require "../src/wopkg/completions"

describe Wopkg::Completions do
  it "every shell lists the commands and asks the index for names" do
    %w(zsh bash fish pwsh).each do |sh|
      s = Wopkg::Completions.for(sh) || raise("no completions for #{sh}")
      Wopkg::Completions::VERBS.each { |v| s.should contain(v) }
      s.should contain("wopkg __complete")
    end
    Wopkg::Completions.for("tcsh").should be_nil
    # `wopkg shard` (curated Crystal shards) is a command like the others.
    Wopkg::Completions::VERBS.should contain("shard")
  end

  it "parses in each shell that is installed" do
    {"bash" => ["bash", "-n"], "zsh" => ["zsh", "-n"], "fish" => ["fish", "--no-execute"]}.each do |sh, argv|
      next unless Process.find_executable(argv[0])
      f = File.tempname("wopkg-comp")
      File.write(f, Wopkg::Completions.for(sh) || raise("no completions for #{sh}"))
      st = Process.run(argv[0], argv[1..] + [f], error: STDERR)
      st.success?.should be_true
      File.delete(f)
    end
  end

  it "pwsh parses it" do
    next unless Process.find_executable("pwsh")
    f = File.tempname("wopkg-comp", ".ps1")
    File.write(f, Wopkg::Completions.pwsh)
    probe = "$e = $null; $null = [System.Management.Automation.Language.Parser]::ParseFile('#{f}', [ref] $null, [ref] $e); exit $e.Count"
    Process.run("pwsh", ["-NoProfile", "-NonInteractive", "-Command", probe], error: STDERR).success?.should be_true
    File.delete(f)
  end

  it "pwsh completes commands and names from the helper" do
    next unless Process.find_executable("pwsh")
    dir = File.tempname("wopkg-fake")
    Dir.mkdir_p(dir)
    File.write("#{dir}/wopkg", "#!/bin/sh\n[ \"$1\" = __complete ] && printf 'firefox\\nfirefox-i18n-de\\n'\n")
    File.chmod("#{dir}/wopkg", 0o755)
    File.write("#{dir}/comp.ps1", Wopkg::Completions.pwsh)
    probe = <<-PS
      . '#{dir}/comp.ps1'
      foreach ($line in 'wopkg install fire', 'wopkg ins', 'wopkg channel b', 'wopkg -S fire', 'wopkg sha') {
          (TabExpansion2 -inputScript $line -cursorColumn $line.Length).CompletionMatches.CompletionText -join ' '
      }
      PS
    buf = IO::Memory.new
    Process.run("pwsh", ["-NoProfile", "-NonInteractive", "-Command", probe], env: {"PATH" => "#{dir}:#{ENV["PATH"]}"}, output: buf, error: STDERR)
    buf.to_s.lines.map(&.strip).should eq(["firefox firefox-i18n-de", "install", "beta", "firefox firefox-i18n-de", "shard"])
  end

  it "bash completes names from the helper" do
    next unless Process.find_executable("bash")
    dir = File.tempname("wopkg-fake")
    Dir.mkdir_p(dir)
    File.write("#{dir}/wopkg", "#!/bin/sh\n[ \"$1\" = __complete ] && printf 'firefox\\nfirefox-i18n-de\\n'\n")
    File.chmod("#{dir}/wopkg", 0o755)
    script = Wopkg::Completions.bash + %(COMP_WORDS=(wopkg install fire); COMP_CWORD=2; _wopkg; echo "${COMPREPLY[*]}"\n)
    File.write("#{dir}/probe.sh", script)
    buf = IO::Memory.new
    Process.run("bash", ["#{dir}/probe.sh"], env: {"PATH" => "#{dir}:/usr/bin:/bin"}, output: buf)
    buf.to_s.strip.should eq("firefox firefox-i18n-de")
  end
end
