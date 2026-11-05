# SPDX-License-Identifier: GPL-2.0-or-later
# Needs hyprland headers (pkg-config hyprland). Builds against librealtime.a.
CXX ?= c++
# -fno-access-control: reads compositor privates (xdg-decoration map) to spot client-drawn title bars.
# RISK: the same flag lets snapping write the drag controller's private grab state (m_dragHotspot,
# m_beginDrag*, plugins/wol-hypr/main.cpp unsnapForDrag) so a snapped window dragged away keeps the
# pointer on its bar. A compositor upgrade that renames or reshapes them breaks the build (or, worse,
# the drag): re-check them against the new headers every time the compositor version changes.
# The compositor's own headers are -isystem: -Wall -Wextra -Werror hold for this plugin's code, not theirs.
DEPS = $(shell pkg-config --cflags hyprland pixman-1 libdrm libinput wayland-server xkbcommon lua-5.5 | sed 's/-I/-isystem /g')
CXXFLAGS += -std=c++2b -fPIC -O2 -Wall -Wextra -Werror -fno-access-control $(DEPS)
RT = ../../target/release/librealtime.a

all: wol-hypr.so

$(RT):
	cargo build --release -p realtime --manifest-path ../../Cargo.toml

wol-hypr.so: main.cpp $(RT)
	$(CXX) $(CXXFLAGS) -shared main.cpp $(RT) -o $@

clean:
	rm -f wol-hypr.so

.PHONY: all clean
