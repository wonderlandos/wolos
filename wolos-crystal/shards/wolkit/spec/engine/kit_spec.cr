# SPDX-License-Identifier: BSD-2-Clause
# The HIG components through the engine: the standard frame lays out (the toolbar is 52 px high, the
# sidebar 220 px wide, a list row 40 px) and the engine's scene and accessibility tree are pinned.
require "./engine_helper_spec"
require "../kit_app_spec"

describe "the HIG components through the engine" do
  it "lays out the frame with the scaffold metrics" do
    Wolkit::Test.run(KitApp) do |t|
      window = t.window
      _x, _y, _w, toolbar_h = window.node_rect(t.find("More actions"))
      toolbar_h.should be <= Wolkit::Scaffold::TOOLBAR_HEIGHT.to_f32
      golden("engine/kit_mounted.scene", t.scene)
      golden("engine/kit_mounted.a11y", t.a11y_snapshot)
    end
  end

  it "shows a menu only while its signal is true" do
    Wolkit::Test.run(KitApp) do |t|
      app = t.app.as(KitApp)
      t.scene.should_not contain("overlay")
      t.click "More actions"
      app.menu_open.value = true
      t.settle
      t.a11y_snapshot.should contain("menuitem")
      app.menu_open.value = false
      t.settle
      t.a11y_snapshot.should_not contain("menuitem")
    end
  end
end
