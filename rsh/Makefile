# SPDX-License-Identifier: GPL-2.0-or-later
# rsh: Ruby with command lines (docs/modules/rsh.md). The interpreter is fetched at one pinned
# commit and checked by sha256 (never kept in the tree); MRUBY_TARBALL= hands in a local copy
# (the package build is offline). Needs: a C/C++ compiler, ruby and bison (the interpreter's build
# script), libpcre2-8 (Regexp).
MRUBY_COMMIT = 831da26b9021de0369d17b71b5667e2941a1a32d
MRUBY_SHA256 = e4fb1bbad8276878e84e3c3b6161ac35a4216853de404f8463675aa2d1920b4f
MRUBY_URL    = https://github.com/mruby/mruby/archive/$(MRUBY_COMMIT).tar.gz
MRUBY_TARBALL ?= build/mruby-$(MRUBY_COMMIT).tar.gz

PREFIX ?= /usr/local
DESTDIR ?=
CXX ?= c++
CC ?= cc
# LINK=static: one file with no shared libraries (needs libpcre2-8.a); LINK=dynamic (the package):
# glibc and libpcre2-8 shared, the C++ runtime linked in.
LINK ?= static
VERSION := $(shell cat VERSION)

M      = build/mruby
MLIB   = $(M)/build/rsh/lib/libmruby.a
OPT ?= -Os
CXXFLAGS += $(OPT) -std=c++20 -Wall -Wextra -Werror -fno-exceptions -fno-rtti -fno-threadsafe-statics -ffunction-sections \
	-fdata-sections -I$(M)/include -I$(M)/build/rsh/include -DMRB_UTF8_STRING -DMRB_STR_LENGTH_MAX=0 -DRSH_VERSION='"$(VERSION)"'
PCRE   = $(shell pkg-config --cflags libpcre2-8 2>/dev/null) -DPCRE2_CODE_UNIT_WIDTH=8
ifeq ($(LINK),static)
LDFLAGS += -static
endif
LDFLAGS += -static-libstdc++ -static-libgcc -Wl,--gc-sections -s
LIBS    = -lpcre2-8 -lm

SRC  = $(wildcard src/*.cpp)
MRBC = $(M)/build/rsh/bin/mrbc
OBJ  = $(SRC:src/%.cpp=build/obj/%.o)
LIBOBJ = $(filter-out build/obj/main.o,$(OBJ))

all: build/rsh

$(MRUBY_TARBALL):
	mkdir -p build
	curl -fsSL -o $@.part $(MRUBY_URL)
	mv $@.part $@

build/mruby/.ok: $(MRUBY_TARBALL) build_config.rb
	echo "$(MRUBY_SHA256)  $(MRUBY_TARBALL)" | sha256sum -c -
	rm -rf $(M) && mkdir -p $(M)
	tar xzf $(MRUBY_TARBALL) -C $(M) --strip-components=1
	touch $@

$(MLIB): build/mruby/.ok
	cd $(M) && MRUBY_CONFIG=$(CURDIR)/build_config.rb ruby ./minirake -j8 all >/dev/null

build/obj/%.o: src/%.cpp $(wildcard src/*.hpp) $(MLIB)
	@mkdir -p build/obj
	$(CXX) $(CXXFLAGS) $(PCRE) -c $< -o $@

# The Ruby half of rsh, compiled to bytecode and linked in (no parsing at start-up).
build/prelude.c: src/prelude.rb $(MLIB)
	$(MRBC) -Brsh_prelude -o $@ src/prelude.rb

build/obj/prelude.o: build/prelude.c
	@mkdir -p build/obj
	$(CC) -Os -I$(M)/include -I$(M)/build/rsh/include -c $< -o $@

build/rsh: $(OBJ) build/obj/prelude.o $(MLIB)
	$(CXX) $(CXXFLAGS) -o $@ $(OBJ) build/obj/prelude.o $(MLIB) $(LDFLAGS) $(LIBS)

build/unit: tests/unit.cpp $(LIBOBJ) build/obj/prelude.o $(MLIB)
	$(CXX) $(CXXFLAGS) $(PCRE) -Isrc -o $@ tests/unit.cpp $(LIBOBJ) build/obj/prelude.o $(MLIB) $(LDFLAGS) $(LIBS)

test: build/unit build/rsh
	./build/unit
	RSH=$(CURDIR)/build/rsh sh tests/run.sh
	@if command -v ruby >/dev/null; then ruby tests/interactive.rb build/rsh; else echo 'interactive: ruby missing, skipped'; fi

bench: build/rsh
	sh tests/bench.sh build/rsh

install: build/rsh
	install -Dm755 build/rsh $(DESTDIR)$(PREFIX)/bin/rsh
	install -Dm644 rsh.1 $(DESTDIR)$(PREFIX)/share/man/man1/rsh.1

clean:
	rm -rf build/obj build/rsh build/unit

distclean:
	rm -rf build

.PHONY: all test bench install clean distclean
