# SPDX-License-Identifier: GPL-2.0-or-later
# rsh: the WolOS shell, Ruby with command lines (rsh/ of github.com/wonderlandos/wolos). One program
# with its interpreter built in.
pkgname=rsh
pkgver=0.1.0  # version: of rsh/VERSION at _wolos_commit
pkgrel=1
pkgdesc='WolOS shell: Ruby with command lines, for scripts and the terminal'
arch=(x86_64)
url='https://github.com/wonderlandos/wolos'
license=(GPL-2.0-or-later MIT)   # MIT: the built-in interpreter
depends=(glibc pcre2)
makedepends=(git ruby bison gcc make)   # ruby runs the interpreter's build script only
options=(!debug)
_wolos_commit=<bumped by bump-wolos.sh>
_mruby_commit=831da26b9021de0369d17b71b5667e2941a1a32d
source=("wolos::git+https://github.com/wonderlandos/wolos.git#commit=$_wolos_commit"
        "mruby-$_mruby_commit.tar.gz::https://github.com/mruby/mruby/archive/$_mruby_commit.tar.gz")
noextract=("mruby-$_mruby_commit.tar.gz")
sha256sums=(SKIP e4fb1bbad8276878e84e3c3b6161ac35a4216853de404f8463675aa2d1920b4f)

build() {
  make -C "$srcdir/wolos/rsh" LINK=dynamic MRUBY_TARBALL="$srcdir/mruby-$_mruby_commit.tar.gz"
}
check() {
  make -C "$srcdir/wolos/rsh" LINK=dynamic MRUBY_TARBALL="$srcdir/mruby-$_mruby_commit.tar.gz" test
}
package() {
  make -C "$srcdir/wolos/rsh" LINK=dynamic MRUBY_TARBALL="$srcdir/mruby-$_mruby_commit.tar.gz" \
    install DESTDIR="$pkgdir" PREFIX=/usr
  install -Dm644 "$srcdir/wolos/rsh/build/mruby/LICENSE" "$pkgdir/usr/share/licenses/$pkgname/LICENSE.interpreter"
}
