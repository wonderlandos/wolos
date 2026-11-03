// SPDX-License-Identifier: GPL-2.0-or-later
// rsh: the line editor for interactive use (raw terminal, history, completion, colours).
#pragma once
#include <string>
#include <vector>

namespace rsh {

struct Editor {
    std::vector<std::string> history;
    std::string history_file;
    // Colours one input line for display (same visible width as the text).
    std::string (*paint)(const std::string& line) = nullptr;
    // Candidates for the word that starts at `start` of `line`.
    std::vector<std::string> (*complete)(const std::string& line, size_t start) = nullptr;

    void load();
    void add(const std::string& entry);
    // false at end of input (Ctrl-D on an empty line).
    bool read(const std::string& prompt, std::string& out);
};

size_t visible_width(const std::string& s);

}  // namespace rsh
