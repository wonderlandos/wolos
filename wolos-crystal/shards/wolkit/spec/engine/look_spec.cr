# SPDX-License-Identifier: BSD-2-Clause
# The default look through the engine: every engine window starts with the kit table, so a plain
# button has the kit's padding (12 px each side) and an inline property overrides it; the DSL's
# variant classes carry the ids of kit_classes.cr; and the look follows dark mode.
require "./engine_helper_spec"

class LookApp < Wolkit::App
  def view
    window "Look", width: 300, height: 120 do
      row gap: :sm, pad: :md, align: :start do
        button "Go", label: "with padding"
        button "Go", label: "without padding", pad_left: :none, pad_right: :none
        button "Main", primary: true
        button "Stop", danger: true, flat: true
      end
    end
  end
end

describe "the default look" do
  it "gives a button the kit's padding, and inline properties win" do
    Wolkit::Test.run(LookApp) do |t|
      window = t.window
      _x, _y, with_w, _h = window.node_rect(t.find("with padding"))
      _x, _y, without_w, _h = window.node_rect(t.find("without padding"))
      # The same word: the kit's padding (12 px a side) is the whole difference.
      (with_w - without_w).should eq 24.0_f32
    end
  end

  it "puts the kit class ids in SET_STYLE" do
    Wolkit::Test.run(LookApp) do |t|
      scene = t.scene
      scene.should contain("button classes=#{Wolkit::KitClass::PRIMARY} ")
      scene.should contain("classes=#{Wolkit::KitClass::DANGER},#{Wolkit::KitClass::FLAT} ")
    end
  end

  it "follows dark mode" do
    Wolkit::Test.run(LookApp) do |t|
      light = t.window.render_rgba[0][0, 4].dup
      t.engine.set_environment(true, 0x7c5cff_u32)
      t.settle
      dark = t.window.render_rgba[0][0, 4].dup
      dark.should_not eq light
    end
  end
end
