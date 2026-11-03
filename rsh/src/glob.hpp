// SPDX-License-Identifier: GPL-2.0-or-later
// rsh: file name patterns. A backslash makes the next character plain (quoted text and Ruby
// values are escaped before they reach here, so they never match as patterns).
#pragma once
#include <string>
#include <vector>

namespace rsh {

std::string glob_escape(const std::string& s);
std::string glob_unescape(const std::string& s);
bool glob_has_meta(const std::string& pat);  // an unescaped * ? or [
// a{b,c}d -> abd acd (only when a { holds an unescaped comma at its own level)
std::vector<std::string> brace_expand(const std::string& pat);
// Sorted matches; `**` is any number of folders; names starting with '.' only match a pattern
// that starts with '.'.
std::vector<std::string> glob(const std::string& pat);
// Does `name` match `pat` (one path segment, same rules)?
bool glob_match(const std::string& pat, const std::string& name);

}  // namespace rsh
