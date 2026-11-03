// SPDX-License-Identifier: GPL-2.0-or-later
// rsh: script text -> Ruby text. Each statement whose first word is not something Ruby knows is a
// command line (docs/modules/rsh.md, "How rsh reads a line"); it becomes one call of the runner
// with its Ruby values, on the same line, so messages point at the right place.
#pragma once
#include <set>
#include <string>
#include <vector>

#include "ast.hpp"

namespace rsh {

struct Mark {
    int line;
    char kind;  // 'C' command line, 'X' capture
    std::string text;
};

struct Diag {
    int line;
    std::string msg;
    bool warning;
};

// Names known to be Ruby while reading (kept between interactive inputs).
struct Known {
    std::set<std::string> locals;
    std::set<std::string> methods;
};

struct Result {
    std::string ruby;
    std::vector<Mark> marks;
    std::vector<Diag> diags;  // errors stop the file; warnings are for -n and --lint
    bool ok() const;
};

// Ruby words that are never commands (besides keywords, constants, locals and defined methods).
extern const char* const RUBY_WORDS[];
bool is_keyword(const std::string& w);
bool is_ruby_word(const std::string& w);

// Methods defined with `def` in `src` (and, when `dir` is given, in the files it loads with
// require_relative "<literal>").
void collect_defs(const std::string& src, const std::string& dir, std::set<std::string>& out, int depth = 0);

Result transpile(const std::string& src, const std::string& file, Known& known);

// Parses `text` as one command line (no Ruby around it) into the table; values are Ruby source
// texts. Used by the runner for `system` and by tests.
int parse_command(const std::string& text, const std::string& file, int line, std::vector<std::string>& values,
                  std::string& err);

}  // namespace rsh
