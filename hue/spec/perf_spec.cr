# SPDX-License-Identifier: GPL-2.0-or-later
# A 50-megapixel JPEG (8660 x 5774) from decode to theme in under 1.5 s with the release binary.
# Runs with WOL_HUE_BENCH=1 (`make spec` sets it); a package build on a loaded builder skips it.
require "./spec_helper"

describe "a 50 MP picture" do
  next unless ENV["WOL_HUE_BENCH"]? == "1"
  next unless bin = ENV["WOL_HUE_BIN"]?

  it "becomes a theme in under 1.5 s" do
    big = File.join(Dir.tempdir, "wol-hue-50mp.jpg")
    unless File.exists?(big)
      src = Hue::LibPixbuf.new_from_file(SpecHelp.scene_files["night-city"], nil)
      huge = Hue::LibPixbuf.scale_simple(src, 8660, 5774, 2) # GDK_INTERP_BILINEAR
      Hue::LibPixbuf.save(huge, big, "jpeg", nil, "quality".to_unsafe, "92".to_unsafe, Pointer(UInt8).null).should eq(1)
      Hue::LibPixbuf.unref(huge.as(Void*))
      Hue::LibPixbuf.unref(src.as(Void*))
    end
    s = SpecHelp.tmpdir
    env = {"XDG_CONFIG_HOME" => s, "XDG_CACHE_HOME" => s} of String => String?
    st = nil
    took = Time.measure do
      st = Process.run(bin, ["from-wallpaper", big, "--no-apply", "--no-cache", "--json"], env: env, output: Process::Redirect::Close, error: Process::Redirect::Inherit)
    end
    st.not_nil!.success?.should be_true
    STDERR.puts "50 MP: #{took.total_milliseconds.round} ms"
    took.total_seconds.should be < 1.5
  end
end
