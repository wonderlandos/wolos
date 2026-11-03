# SPDX-License-Identifier: BSD-2-Clause
# Compile-fail specs: every file in spec/compile_errors/ (`*.crfail`, so the formatter and the linter leave them alone) is a small program that must not compile.
# Its header says what the compiler must print (`# expect:`) and the line that ends in `# <= error`
# is where the compiler must point, so the messages the plan promises (section 3.7) are pinned, with the user's file and
# line, and a macro change that loses either fails here.
require "./spec_helper"

describe "compile errors" do
  Dir.glob("spec/compile_errors/*.crfail").sort.each do |path|
    lines = File.read_lines(path)
    expect = lines.find(&.starts_with?("# expect: ")).try(&.lchop("# expect: ")) || raise "#{path}: no # expect: line"
    line = lines.index(&.ends_with?("# <= error")).try(&.+(1)) || raise "#{path}: no line ends in # <= error"

    it "#{File.basename(path, ".crfail")} says: #{expect}" do
      output = IO::Memory.new
      status = Process.run("crystal", ["build", "--no-codegen", "--no-color", path], output: output, error: output)
      status.success?.should be_false
      text = output.to_s
      text.should contain("Error: #{expect}")
      text.should match(/(In|Code in) #{Regex.escape(path)}:#{line}:/)
    end
  end
end
