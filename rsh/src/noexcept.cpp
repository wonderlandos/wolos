// SPDX-License-Identifier: GPL-2.0-or-later
// rsh is built without C++ exceptions. The standard containers still name these helpers for
// impossible cases (a string longer than memory, an index past the end); ending the process
// here keeps the runtime's exception and name-printing code out of the binary (about 0.3 MB).
#include <cstdio>
#include <cstdlib>
#include <new>

namespace {
[[noreturn]] void die(const char* what) {
    std::fprintf(stderr, "rsh: internal error: %s\n", what);
    std::abort();
}
}  // namespace

namespace std {
void __throw_bad_alloc() { die("out of memory"); }
void __throw_bad_array_new_length() { die("bad array length"); }
void __throw_length_error(const char* s) { die(s); }
void __throw_logic_error(const char* s) { die(s); }
void __throw_out_of_range_fmt(const char* s, ...) { die(s); }
}  // namespace std

void* operator new(std::size_t n) {
    void* p = std::malloc(n ? n : 1);
    if (!p) die("out of memory");
    return p;
}
void operator delete(void* p) noexcept { std::free(p); }
void operator delete(void* p, std::size_t) noexcept { std::free(p); }
