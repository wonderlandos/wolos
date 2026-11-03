# SPDX-License-Identifier: BSD-2-Clause
# Specs for the Navigator and `switch`: push, back and replace rebuild the screen.
require "./spec_helper"

record Detail, id : Int32
alias Route = Symbol | Detail

class NavApp < Wolkit::App
  getter nav = Wolkit::Navigator(Route).new(:list)

  def view
    window "Nav" do
      column do
        navigator @nav do |route|
          case route
          in Symbol
            text "list screen"
          in Detail
            text "detail #{route.id}"
          end
        end
      end
    end
  end
end

describe Wolkit::Navigator do
  after_each { Wolkit::Test.reset }

  it "shows the top route and frees the old screen" do
    app = NavApp.new
    recorder = Wolkit::Test.record(app)
    window = recorder.windows.first
    window.ops.should contain("SET_TEXT 3 \"list screen\"")
    app.nav.push(Detail.new(7))
    Wolkit::Test.settle
    window.last_ops.should contain("SET_TEXT 4 \"detail 7\"")
    window.last_ops.any?(&.starts_with?("FREE 3")).should be_true
    app.nav.depth.should eq 2
    app.nav.back
    Wolkit::Test.settle
    app.nav.depth.should eq 1
    window.last_ops.any?(&.includes?("\"list screen\"")).should be_true
    app.nav.back
    app.nav.depth.should eq 1
    app.nav.can_go_back?.should be_false
  end

  it "replaces the top route" do
    app = NavApp.new
    recorder = Wolkit::Test.record(app)
    app.nav.push(Detail.new(1))
    app.nav.replace(Detail.new(2))
    Wolkit::Test.settle
    app.nav.depth.should eq 2
    recorder.windows.first.last_ops.any?(&.includes?("detail 2")).should be_true
  end
end
