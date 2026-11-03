# SPDX-License-Identifier: BSD-2-Clause
# The counter in a headless engine: click the buttons, read the number.
require "spec"
require "../../../shards/wolkit/src/wolkit/test"
require "../src/counter"

describe Counter do
  it "counts up and down" do
    Wolkit::Test.run(Counter) do |t|
      t.text("0").should eq "0"
      t.click "Increase"
      t.click "Increase"
      t.click "Decrease"
      t.text("1").should eq "1"
    end
  end
end
