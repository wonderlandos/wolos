# SPDX-License-Identifier: GPL-2.0-or-later
# rsh: Ruby with a shell layer, as a program and a terminal shell. One program with its
# interpreter built in. Builds the tagged release of github.com/wonderlandos/rsh; the same text
# is `rsh/PKGBUILD` of wolos-pkgbuilds (proposed text, in the WolOS repository as docs/modules/rsh.PKGBUILD).
pkgname=rsh
pkgver=0.1.3
pkgrel=1
pkgdesc='Ruby with a shell layer: scripts, plain Ruby files and a terminal shell'
arch=(x86_64)
url='https://github.com/wonderlandos/rsh'
license=(GPL-2.0-or-later MIT)   # MIT: the built-in interpreter
depends=(glibc pcre2)
makedepends=(ruby bison gcc make)   # ruby runs the interpreter's build script only
options=(!debug)
_mruby_commit=831da26b9021de0369d17b71b5667e2941a1a32d
source=("$pkgname-$pkgver.tar.gz::$url/archive/refs/tags/v$pkgver.tar.gz"
        "mruby-$_mruby_commit.tar.gz::https://github.com/mruby/mruby/archive/$_mruby_commit.tar.gz")
noextract=("mruby-$_mruby_commit.tar.gz")
sha256sums=('6ccb6597707d6d174f0210eb87d7db5b1ab2c97bf75e7f31a0cb5463f1cb1433'
            'e4fb1bbad8276878e84e3c3b6161ac35a4216853de404f8463675aa2d1920b4f')

build() {
  make -C "$pkgname-$pkgver" LINK=dynamic MRUBY_TARBALL="$srcdir/mruby-$_mruby_commit.tar.gz"
}
check() {
  make -C "$pkgname-$pkgver" LINK=dynamic MRUBY_TARBALL="$srcdir/mruby-$_mruby_commit.tar.gz" test
}
package() {
  make -C "$pkgname-$pkgver" LINK=dynamic MRUBY_TARBALL="$srcdir/mruby-$_mruby_commit.tar.gz" \
    install DESTDIR="$pkgdir" PREFIX=/usr
  install -Dm644 "$pkgname-$pkgver/build/mruby/LICENSE" "$pkgdir/usr/share/licenses/$pkgname/LICENSE.interpreter"
  install -Dm644 "$pkgname-$pkgver/LICENSE" "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
}
