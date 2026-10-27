# Needs hyprland headers (pkg-config hyprland). Builds against librealtime.a.
CXX ?= c++
# -fno-access-control: reads Hyprland privates (xdg-decoration map) to spot client-drawn title bars.
CXXFLAGS += -std=c++2b -fPIC -O2 -fno-access-control $(shell pkg-config --cflags hyprland pixman-1 libdrm libinput wayland-server xkbcommon lua-5.5)
RT = ../../target/release/librealtime.a

all: wol-hypr.so

$(RT):
	cargo build --release -p realtime --manifest-path ../../Cargo.toml

wol-hypr.so: main.cpp $(RT)
	$(CXX) $(CXXFLAGS) -shared main.cpp $(RT) -o $@

clean:
	rm -f wol-hypr.so

.PHONY: all clean
