# SPDX-License-Identifier: BSD-2-Clause
# Style sheets through the engine: a compiled sheet is used for the open windows, and a sheet the
# compiler refuses says why (with the file position) instead of crashing the app.
require "./engine_helper_spec"
require "../dsl_app_spec"

describe "style sheets" do
  it "uses a compiled sheet for the windows that are open" do
    Wolkit::Test.run(SpecTodos) do |t|
      sheet = Wolkit::FFI::StyleTable.compile(".greeting { font-size: title; color: accent; }")
      sheet.size.should be > 24
      String.new(sheet[0, 4]).should eq "WSB1"
      Wolkit.use_styles(sheet)
      t.settle
      t.scene.should contain("text_input")
    end
  end

  it "reports a refused sheet with its position and a way out" do
    error = expect_raises(Wol::InvalidArgument) { Wolkit::FFI::StyleTable.compile(".a { background: #ff0000; }") }
    error.message.to_s.should contain("style.wolcss:1:18")
    error.message.to_s.should contain("colours here are theme tokens")
  end
end
