// SPDX-License-Identifier: GPL-2.0-or-later
#include "glob.hpp"

#include <dirent.h>
#include <fnmatch.h>
#include <sys/stat.h>

#include <algorithm>
#include <cstring>

namespace rsh {

std::string glob_escape(const std::string& s) {
    std::string o;
    o.reserve(s.size());
    for (char c : s) {
        if (std::strchr("*?[]{},\\", c)) o += '\\';
        o += c;
    }
    return o;
}

std::string glob_unescape(const std::string& s) {
    std::string o;
    for (size_t i = 0; i < s.size(); ++i) {
        if (s[i] == '\\' && i + 1 < s.size()) ++i;
        o += s[i];
    }
    return o;
}

bool glob_has_meta(const std::string& p) {
    for (size_t i = 0; i < p.size(); ++i) {
        if (p[i] == '\\') {
            ++i;
            continue;
        }
        if (p[i] == '*' || p[i] == '?') return true;
        if (p[i] == '[' && p.find(']', i + 1) != std::string::npos) return true;
    }
    return false;
}

std::vector<std::string> brace_expand(const std::string& p) {
    for (size_t i = 0; i < p.size(); ++i) {
        if (p[i] == '\\') {
            ++i;
            continue;
        }
        if (p[i] != '{') continue;
        int depth = 0;
        std::vector<size_t> commas;
        size_t close = std::string::npos;
        for (size_t j = i + 1; j < p.size(); ++j) {
            if (p[j] == '\\') {
                ++j;
                continue;
            }
            if (p[j] == '{') ++depth;
            else if (p[j] == '}') {
                if (depth == 0) {
                    close = j;
                    break;
                }
                --depth;
            } else if (p[j] == ',' && depth == 0) commas.push_back(j);
        }
        if (close == std::string::npos) return {p};
        if (commas.empty()) continue;
        std::string pre = p.substr(0, i), post = p.substr(close + 1);
        std::vector<std::string> out;
        size_t b = i + 1;
        commas.push_back(close);
        for (size_t c : commas) {
            for (auto& e : brace_expand(pre + p.substr(b, c - b) + post)) out.push_back(e);
            b = c + 1;
        }
        return out;
    }
    return {p};
}

bool glob_match(const std::string& pat, const std::string& name) {
    return fnmatch(pat.c_str(), name.c_str(), FNM_PERIOD) == 0;
}

namespace {

std::string join(const std::string& base, const std::string& name) {
    if (base.empty()) return name;
    if (base == "/") return "/" + name;
    return base + "/" + name;
}

bool is_dir(const std::string& path, const dirent* d) {
    if (d && d->d_type != DT_UNKNOWN && d->d_type != DT_LNK) return d->d_type == DT_DIR;
    struct stat st;
    return stat(path.c_str(), &st) == 0 && S_ISDIR(st.st_mode);
}

void walk(const std::string& base, const std::vector<std::string>& segs, size_t k, std::vector<std::string>& out) {
    if (k == segs.size()) {
        out.push_back(base);
        return;
    }
    const std::string& seg = segs[k];
    bool last = k + 1 == segs.size();
    if (seg == "**") {
        walk(base, segs, k + 1, out);
        DIR* d = opendir(base.empty() ? "." : base.c_str());
        if (!d) return;
        std::vector<std::string> subs;
        while (dirent* e = readdir(d)) {
            if (e->d_name[0] == '.') continue;
            std::string path = join(base, e->d_name);
            if (e->d_type == DT_DIR || (e->d_type == DT_UNKNOWN && is_dir(path, nullptr))) subs.push_back(path);
        }
        closedir(d);
        for (auto& s : subs) walk(s, segs, k, out);
        return;
    }
    if (!glob_has_meta(seg)) {
        std::string path = join(base, glob_unescape(seg));
        struct stat st;
        if (last) {
            if (lstat(path.c_str(), &st) == 0) out.push_back(path);
        } else if (stat(path.c_str(), &st) == 0 && S_ISDIR(st.st_mode))
            walk(path, segs, k + 1, out);
        return;
    }
    DIR* d = opendir(base.empty() ? "." : base.c_str());
    if (!d) return;
    std::vector<std::pair<std::string, bool>> hits;
    while (dirent* e = readdir(d)) {
        if (!std::strcmp(e->d_name, ".") || !std::strcmp(e->d_name, "..")) continue;
        if (!glob_match(seg, e->d_name)) continue;
        std::string path = join(base, e->d_name);
        hits.emplace_back(path, last ? false : is_dir(path, e));
    }
    closedir(d);
    for (auto& [path, dir] : hits) {
        if (last) out.push_back(path);
        else if (dir) walk(path, segs, k + 1, out);
    }
}

}  // namespace

std::vector<std::string> glob(const std::string& pat) {
    std::vector<std::string> segs;
    std::string base;
    size_t i = 0;
    if (!pat.empty() && pat[0] == '/') {
        base = "/";
        i = 1;
    }
    while (i <= pat.size()) {
        size_t e = pat.find('/', i);
        if (e == std::string::npos) e = pat.size();
        if (e > i) segs.push_back(pat.substr(i, e - i));
        i = e + 1;
    }
    std::vector<std::string> out;
    walk(base, segs, 0, out);
    std::sort(out.begin(), out.end());
    out.erase(std::unique(out.begin(), out.end()), out.end());
    return out;
}

}  // namespace rsh
