# SPDX-License-Identifier: GPL-2.0-or-later
# Recorded palettes (spec/fixtures/golden.json; `make golden`): 10 scenes x dark/light x 3 modes,
# every colour within OKLab deltaE 0.02 of the record.
require "./spec_helper"

include Hue

GOLDEN_MODES = %w(normal monochromatic pastel)

describe "golden palettes" do
  it "match the recorded ones within deltaE 0.02" do
    g = JSON.parse(File.read(File.join(__DIR__, "fixtures", "golden.json")))
    cases = g["cases"].as_h
    cases.size.should eq(10 * 2 * GOLDEN_MODES.size)
    worst = 0.0
    cases.each do |key, want|
      name, variant, mode = key.split('/')
      got = SpecHelp.palette(name, variant == "light", mode)
      want.as_a.each_with_index do |w, i|
        d = Color.delta_e(got[i], w.as_s)
        worst = Math.max(worst, d)
        fail "#{key} slot #{i}: #{got[i]} vs #{w.as_s} (deltaE #{d.round(4)})" if d > 0.02
      end
    end
    worst.should be <= 0.02
  end
end
