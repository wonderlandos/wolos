require "spec"
require "../src/wopkg/completions"

describe Wopkg::Completions do
  it "every shell lists the commands and asks the index for names" do
    %w(zsh bash fish).each do |sh|
      s = Wopkg::Completions.for(sh).not_nil!
      Wopkg::Completions::VERBS.each { |v| s.should contain(v) }
      s.should contain("wopkg __complete")
    end
    Wopkg::Completions.for("tcsh").should be_nil
  end

  it "parses in each shell that is installed" do
    {"bash" => ["bash", "-n"], "zsh" => ["zsh", "-n"], "fish" => ["fish", "--no-execute"]}.each do |sh, argv|
      next unless Process.find_executable(argv[0])
      f = File.tempname("wopkg-comp")
      File.write(f, Wopkg::Completions.for(sh).not_nil!)
      st = Process.run(argv[0], argv[1..] + [f], error: STDERR)
      st.success?.should be_true
      File.delete(f)
    end
  end

  it "bash completes names from the helper" do
    next unless Process.find_executable("bash")
    dir = File.tempname("wopkg-fake")
    Dir.mkdir_p(dir)
    File.write("#{dir}/wopkg", "#!/bin/sh\n[ \"$1\" = __complete ] && printf 'firefox\\nfirefox-i18n-de\\n'\n")
    File.chmod("#{dir}/wopkg", 0o755)
    script = Wopkg::Completions.bash + %(COMP_WORDS=(wopkg install fire); COMP_CWORD=2; _wopkg; echo "${COMPREPLY[*]}"\n)
    buf = IO::Memory.new
    Process.run("bash", ["-c", script], env: {"PATH" => "#{dir}:/usr/bin:/bin"}, output: buf)
    buf.to_s.strip.should eq("firefox firefox-i18n-de")
  end
end
