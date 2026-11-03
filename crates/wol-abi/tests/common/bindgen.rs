// SPDX-License-Identifier: BSD-2-Clause
//! A C header read into a small model, and the Crystal and .NET declarations written from it
//! (ADR 0020, ADR 0036). The header is the one source: a new function, callback, struct or constant
//! changes what this writes, and `tests/bindings.rs` fails until the checked-in binding files are
//! regenerated (`WOL_BLESS=1 cargo test -p wol-abi --test bindings`). Swift needs no generated file:
//! its module map imports the header itself.
//!
//! One reader serves every header the Crystal shard binds (`wol.h`, `wopkg.h`, `wol_realtime.h`); a
//! [`Target`] says which prefix, lib name and link name each uses. The Crystal text is written the
//! way `crystal tool format` leaves it, so the CI format check passes on generated files unchanged.
#![allow(dead_code)]

/// Header types that are defined outside the header (another library's handles). A header that
/// uses one of these needs a row here, or the generator stops and names this file.
pub const EXTERN_TYPES: &[(&str, &str)] = &[
    // Vulkan handles: a dispatchable handle is a pointer, a non-dispatchable one a 64-bit integer.
    ("VkInstance", "Void*"),
    ("VkSurfaceKHR", "UInt64"),
];

/// Which header is read and how its Crystal `lib` is named and linked.
#[derive(Debug, Clone, Copy)]
pub struct Target {
    /// Header path from the repository root, as shown in the generated banner.
    pub header: &'static str,
    /// Prefix of every exported name (`wol_`); stripped from type names, kept on functions.
    pub prefix: &'static str,
    /// Crystal `lib` name (`LibWol`).
    pub lib: &'static str,
    /// `@[Link(...)]` name, or `None` when the library has no shared object to link (the caller
    /// passes its own `--link-flags`).
    pub link: Option<&'static str>,
    /// SPDX licence of the generated file: the header's.
    pub license: &'static str,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Ty {
    pub base: String,
    pub ptrs: usize,
}

#[derive(Debug, Clone)]
pub struct Func {
    pub name: String,
    pub ret: Ty,
    pub params: Vec<(String, Ty)>,
}

#[derive(Debug, Default)]
pub struct Header {
    /// `#define WOL_X 3` (or `3u`, `0x100u`), in order; the value is the C literal.
    pub consts: Vec<(String, String)>,
    /// `typedef struct wol_x wol_x;`
    pub opaque: Vec<String>,
    /// `typedef struct X { float a; } X;`: passed by value, fields in order.
    pub structs: Vec<(String, Vec<(String, Ty)>)>,
    /// `typedef int32_t wol_status;`
    pub aliases: Vec<(String, String)>,
    /// `typedef void (*wol_click_fn)(...)`.
    pub callbacks: Vec<Func>,
    pub funcs: Vec<Func>,
}

fn strip_comments(src: &str) -> String {
    let mut out = String::new();
    let mut rest = src;
    while let Some(i) = rest.find("/*") {
        out.push_str(&rest[..i]);
        let end = rest[i..].find("*/").expect("unterminated comment") + i + 2;
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

fn ty(s: &str) -> Ty {
    let ptrs = s.matches('*').count();
    let base = s
        .replace('*', " ")
        .split_whitespace()
        .filter(|w| *w != "const")
        .collect::<Vec<_>>()
        .join(" ");
    Ty { base, ptrs }
}

/// `const char *utf8` -> ("utf8", const char *). The name is the trailing identifier.
fn param(s: &str) -> (String, Ty) {
    let s = s.trim();
    let split = s
        .rfind(|c: char| !(c.is_alphanumeric() || c == '_'))
        .map_or(0, |i| i + 1);
    let (t, name) = s.split_at(split);
    assert!(!t.trim().is_empty(), "parameter without a type: {s}");
    (name.to_string(), ty(t))
}

fn params(args: &str) -> Vec<(String, Ty)> {
    let args = args.trim();
    if args.is_empty() || args == "void" {
        return Vec::new();
    }
    args.split(',').map(param).collect()
}

/// `RET name(ARGS)` with RET possibly ending in `*`.
fn func(decl: &str) -> Func {
    let open = decl
        .find('(')
        .unwrap_or_else(|| panic!("not a prototype: {decl}"));
    let close = decl.rfind(')').unwrap();
    let head = decl[..open].trim();
    let (name, ret) = param(head);
    Func {
        name,
        ret,
        params: params(&decl[open + 1..close]),
    }
}

/// `typedef struct X { float a; float b; } X` (the text up to the closing name, no final `;`).
fn struct_body(s: &str) -> (String, Vec<(String, Ty)>) {
    let open = s.find('{').unwrap();
    let close = s.rfind('}').unwrap();
    let tag = s["typedef struct".len()..open].trim();
    let name = s[close + 1..].trim();
    assert!(
        tag.is_empty() || tag == name,
        "struct typedef with a different tag and name: {s}"
    );
    let fields = s[open + 1..close]
        .split(';')
        .map(str::trim)
        .filter(|f| !f.is_empty())
        .map(|f| {
            assert!(
                !f.contains(',') && !f.contains('['),
                "struct field the reader cannot take (one field per declaration, no arrays): {f}"
            );
            param(f)
        })
        .collect();
    (name.to_string(), fields)
}

pub fn parse(src: &str) -> Header {
    let src = strip_comments(src);
    let mut h = Header::default();
    let mut code = String::new();
    for line in src.lines() {
        let t = line.trim();
        if let Some(d) = t.strip_prefix("#define ") {
            let mut it = d.split_whitespace();
            let (name, value) = (it.next().unwrap(), it.next());
            if let Some(v) = value {
                h.consts.push((name.to_string(), v.to_string()));
            }
        } else if t.starts_with('#') || t.starts_with("extern \"C\"") || t == "}" {
        } else {
            code.push_str(t);
            code.push(' ');
        }
    }
    let code = code.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut rest = code.as_str();
    while !rest.trim().is_empty() {
        // A struct body holds `;`s of its own: take it whole, up to the `;` after its `}`.
        let semi = rest.find(';').expect("declaration without a `;`");
        let end = match rest.find('{') {
            Some(b) if b < semi => {
                let close = rest[b..].find('}').expect("unterminated struct") + b;
                rest[close..].find(';').expect("struct without a `;`") + close
            }
            _ => semi,
        };
        let s = rest[..end].trim();
        rest = &rest[end + 1..];
        if s.is_empty() {
            continue;
        }
        if s.starts_with("typedef struct") && s.contains('{') {
            h.structs.push(struct_body(s));
        } else if let Some(t) = s.strip_prefix("typedef ") {
            if let Some(p) = t.find("(*") {
                // typedef RET (*name)(ARGS)
                let q = t[p..].find(')').unwrap() + p;
                let name = t[p + 2..q].trim().to_string();
                let mut f = func(&format!("{}cb{}", &t[..p], &t[q + 1..]));
                f.name = name;
                h.callbacks.push(f);
            } else if let Some(t) = t.strip_prefix("struct ") {
                let n: Vec<&str> = t.split(' ').collect();
                assert_eq!(n[0], n[1], "opaque struct typedef {s}");
                h.opaque.push(n[0].to_string());
            } else {
                let (name, t) = param(t);
                h.aliases.push((name, t.base));
            }
        } else {
            h.funcs.push(func(s));
        }
    }
    h
}

/// `wol_search_provider` -> `SearchProvider`; a name without the prefix (`WolVisual`) keeps its words.
fn camel(prefix: &str, name: &str) -> String {
    name.strip_prefix(prefix)
        .unwrap_or(name)
        .split('_')
        .filter(|w| !w.is_empty())
        .map(|w| w[..1].to_uppercase() + &w[1..])
        .collect()
}

/// A C integer literal as Crystal: `1u` -> `1_u32`, `0x100u` -> `0x100_u32`, `1ull` -> `1_u64`.
/// Anything else (a cast, an expression, a float) is refused rather than guessed.
pub fn crystal_literal(v: &str) -> String {
    let lower = v.to_ascii_lowercase();
    let digits = lower.trim_end_matches(['u', 'l']);
    let suffix = &lower[digits.len()..];
    let ok = if let Some(hex) = digits.strip_prefix("0x") {
        !hex.is_empty() && hex.chars().all(|c| c.is_ascii_hexdigit())
    } else {
        let d = digits.strip_prefix('-').unwrap_or(digits);
        !d.is_empty() && d.chars().all(|c| c.is_ascii_digit())
    };
    assert!(ok, "constant the generator cannot write in Crystal: {v}");
    let digits = &v[..digits.len()];
    match suffix {
        "" => digits.to_string(),
        "u" => format!("{digits}_u32"),
        "l" | "ll" => format!("{digits}_i64"),
        "ul" | "lu" | "ull" | "llu" => format!("{digits}_u64"),
        _ => panic!("constant suffix the generator cannot write in Crystal: {v}"),
    }
}

/// Words a Crystal parameter cannot be named.
const CRYSTAL_KEYWORDS: &[&str] = &[
    "out",
    "in",
    "end",
    "type",
    "self",
    "begin",
    "alias",
    "class",
    "def",
    "do",
    "if",
    "else",
    "module",
    "struct",
    "lib",
    "fun",
    "of",
    "then",
    "when",
    "case",
    "while",
    "until",
    "yield",
    "next",
    "return",
    "nil",
    "true",
    "false",
    "with",
    "select",
    "macro",
    "include",
    "extend",
    "require",
    "ensure",
    "rescue",
    "super",
    "unless",
    "abstract",
    "private",
    "protected",
];

impl Header {
    fn is_alias(&self, n: &str) -> bool {
        self.aliases.iter().any(|(a, _)| a == n)
    }
    fn is_opaque(&self, n: &str) -> bool {
        self.opaque.iter().any(|o| o == n)
    }
    fn is_struct(&self, n: &str) -> bool {
        self.structs.iter().any(|(s, _)| s == n)
    }
    fn is_callback(&self, n: &str) -> bool {
        self.callbacks.iter().any(|c| c.name == n)
    }

    fn crystal_ty(&self, t: &Target, ty: &Ty) -> String {
        let b = ty.base.as_str();
        let base = match b {
            "void" if ty.ptrs == 0 => return "Void".into(),
            "void" => "Void".to_string(),
            "char" | "uint8_t" | "unsigned char" => "UInt8".into(),
            "int8_t" | "signed char" => "Int8".into(),
            "uint16_t" => "UInt16".into(),
            "int16_t" => "Int16".into(),
            "size_t" => "LibC::SizeT".into(),
            "ssize_t" => "LibC::SSizeT".into(),
            "int32_t" | "int" => "Int32".into(),
            "uint32_t" | "unsigned" | "unsigned int" => "UInt32".into(),
            "int64_t" => "Int64".into(),
            "uint64_t" => "UInt64".into(),
            "float" => "Float32".into(),
            "double" => "Float64".into(),
            "bool" | "_Bool" => "Bool".into(),
            n if self.is_opaque(n)
                || self.is_struct(n)
                || self.is_alias(n)
                || self.is_callback(n) =>
            {
                camel(t.prefix, n)
            }
            n => match EXTERN_TYPES.iter().find(|(c, _)| *c == n) {
                Some((_, cr)) => cr.to_string(),
                None => panic!(
                    "no Crystal type for `{n}` in {}: add a row to EXTERN_TYPES in \
                     crates/wol-abi/tests/common/bindgen.rs",
                    t.header
                ),
            },
        };
        format!("{base}{}", "*".repeat(ty.ptrs))
    }

    /// [`Self::crystal_ty`] as seen from outside the `lib`: its own types carry the `Lib::` prefix.
    fn crystal_ty_outside(&self, t: &Target, ty: &Ty) -> String {
        let n = ty.base.as_str();
        let own = self.is_opaque(n) || self.is_struct(n) || self.is_alias(n) || self.is_callback(n);
        let inner = self.crystal_ty(t, ty);
        if own {
            format!("{}::{inner}", t.lib)
        } else {
            inner
        }
    }

    /// The `lib` for Crystal, formatted as `crystal tool format` writes it.
    pub fn crystal(&self, t: &Target) -> String {
        let pname = |n: &str| {
            if CRYSTAL_KEYWORDS.contains(&n) {
                format!("{n}_")
            } else {
                n.to_string()
            }
        };
        let mut s = format!(
            "# SPDX-License-Identifier: {}\n\
             # Generated from {} by crates/wol-abi/tests/bindings.rs; do not edit.\n\
             # Regenerate: WOL_BLESS=1 cargo test -p wol-abi --test bindings\n",
            t.license, t.header
        );
        match t.link {
            Some(l) => s += &format!("@[Link(\"{l}\")]\n"),
            None => {
                s += "# No @[Link]: the library is a static archive; the program passes its own --link-flags.\n"
            }
        }
        s += &format!("lib {}\n", t.lib);

        // Sections are separated by one blank line; empty ones are left out (the formatter would
        // fold two blank lines into one).
        let mut sections: Vec<String> = Vec::new();
        if !self.consts.is_empty() {
            // One aligned block: names padded to the longest, values right-aligned.
            let vals: Vec<(String, String)> = self
                .consts
                .iter()
                .map(|(n, v)| (n.clone(), crystal_literal(v)))
                .collect();
            let nw = vals.iter().map(|(n, _)| n.len()).max().unwrap();
            let vw = vals.iter().map(|(_, v)| v.len()).max().unwrap();
            sections.push(
                vals.iter()
                    .map(|(n, v)| format!("  {n:<nw$} = {v:>vw$}\n"))
                    .collect(),
            );
        }
        if !self.aliases.is_empty() {
            sections.push(
                self.aliases
                    .iter()
                    .map(|(n, a)| {
                        format!(
                            "  alias {} = {}\n",
                            camel(t.prefix, n),
                            self.crystal_ty(t, &ty(a))
                        )
                    })
                    .collect(),
            );
        }
        if !self.opaque.is_empty() {
            let structs: Vec<String> = self
                .opaque
                .iter()
                .map(|n| {
                    format!(
                        "  struct {}\n    opaque : UInt8\n  end\n",
                        camel(t.prefix, n)
                    )
                })
                .collect();
            sections.push(format!(
                "  # Handles are opaque: the field exists only because a Crystal struct cannot be empty.\n{}",
                structs.join("\n")
            ));
        }
        for (n, fields) in &self.structs {
            let mut b = format!("  struct {}\n", camel(t.prefix, n));
            for (f, fty) in fields {
                b += &format!("    {} : {}\n", pname(f), self.crystal_ty(t, fty));
            }
            sections.push(b + "  end\n");
        }
        if !self.callbacks.is_empty() {
            sections.push(
                self.callbacks
                    .iter()
                    .map(|c| {
                        let args: Vec<String> = c
                            .params
                            .iter()
                            .map(|(_, p)| self.crystal_ty(t, p))
                            .collect();
                        format!(
                            "  alias {} = {}-> {}\n",
                            camel(t.prefix, &c.name),
                            if args.is_empty() {
                                String::new()
                            } else {
                                args.join(", ") + " "
                            },
                            self.crystal_ty(t, &c.ret)
                        )
                    })
                    .collect(),
            );
        }
        if !self.funcs.is_empty() {
            sections.push(
                self.funcs
                    .iter()
                    .map(|f| {
                        let args: Vec<String> = f
                            .params
                            .iter()
                            .map(|(n, p)| format!("{} : {}", pname(n), self.crystal_ty(t, p)))
                            .collect();
                        let args = if args.is_empty() {
                            String::new()
                        } else {
                            format!("({})", args.join(", "))
                        };
                        let ret = if f.ret == ty("void") {
                            String::new()
                        } else {
                            format!(" : {}", self.crystal_ty(t, &f.ret))
                        };
                        format!("  fun {}{args}{ret}\n", f.name)
                    })
                    .collect(),
            );
        }
        s + &sections.join("\n") + "end\n"
    }

    /// A spec that takes the address of every function, so a header function the library does not
    /// export fails at link time, before any wrapper calls it. `lib_require` is relative to the spec.
    pub fn crystal_links_spec(&self, t: &Target, lib_require: &str) -> String {
        let mut s = format!(
            "# SPDX-License-Identifier: {}\n\
             # Generated from {} by crates/wol-abi/tests/bindings.rs; do not edit.\n\
             # Regenerate: WOL_BLESS=1 cargo test -p wol-abi --test bindings\n\
             require \"spec\"\n\
             require \"{lib_require}\"\n\n\
             describe \"{}\" do\n  \
             it \"links every function of the header\" do\n    \
             pointers = [\n",
            t.license, t.header, t.lib
        );
        for f in &self.funcs {
            let types: Vec<String> = f
                .params
                .iter()
                .map(|(_, p)| self.crystal_ty_outside(t, p))
                .collect();
            let types = if types.is_empty() {
                String::new()
            } else {
                format!("({})", types.join(", "))
            };
            s += &format!("      (->{}.{}{types}).pointer,\n", t.lib, f.name);
        }
        s += &format!(
            "    ]\n    pointers.size.should eq {}\n    pointers.each(&.null?.should(be_false))\n  end\nend\n",
            self.funcs.len()
        );
        s
    }

    fn cs_ty(&self, t: &Ty) -> String {
        let b = t.base.as_str();
        let one = |p: &str, q: &str| match t.ptrs {
            0 => p.to_string(),
            1 => q.to_string(),
            _ => panic!("no C# type for {t:?}"),
        };
        match b {
            "void" if t.ptrs == 0 => "void".into(),
            "void" => "IntPtr".into(),
            "char" => "byte*".into(),
            "size_t" => one("nuint", "nuint*"),
            "int32_t" | "int" => one("int", "int*"),
            "uint32_t" => one("uint", "uint*"),
            "double" => one("double", "double*"),
            n if self.is_alias(n) => one("int", "int*"),
            n if self.is_callback(n) => n.to_string(),
            n if self.is_opaque(n) => match t.ptrs {
                1 => "IntPtr".into(),
                2 => "IntPtr*".into(),
                _ => panic!("no C# type for {t:?}"),
            },
            n => panic!("no C# type for {n}"),
        }
    }

    /// P/Invoke declarations for .NET (`Wol.Native`), names exactly as in the header.
    pub fn dotnet(&self) -> String {
        let kw = [
            "out", "string", "checked", "default", "class", "fixed", "is", "as", "this", "new",
            "event", "ref", "in", "params", "object", "base", "lock",
        ];
        let id = |n: &str| {
            if kw.contains(&n) {
                format!("@{n}")
            } else {
                n.to_string()
            }
        };
        let mut s = String::from(
            "// SPDX-License-Identifier: BSD-2-Clause\n\
             // Generated from include/wol.h by crates/wol-abi/tests/bindings.rs; do not edit.\n\
             // Regenerate: WOL_BLESS=1 cargo test -p wol-abi --test bindings\n\
             using System;\nusing System.Runtime.InteropServices;\n\nnamespace Wol;\n\n\
             public static unsafe class Native\n{\n    const string Lib = \"libwol.so.1\";\n\n",
        );
        for (n, v) in &self.consts {
            s += &format!("    public const int {n} = {v};\n");
        }
        s.push('\n');
        for c in &self.callbacks {
            let args: Vec<String> = c
                .params
                .iter()
                .map(|(n, t)| format!("{} {}", self.cs_ty(t), id(n)))
                .collect();
            s += &format!(
                "    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]\n    public delegate {} {}({});\n",
                self.cs_ty(&c.ret),
                c.name,
                args.join(", ")
            );
        }
        s.push('\n');
        for f in &self.funcs {
            let args: Vec<String> = f
                .params
                .iter()
                .map(|(n, t)| format!("{} {}", self.cs_ty(t), id(n)))
                .collect();
            s += &format!(
                "    [DllImport(Lib)] public static extern {} {}({});\n",
                self.cs_ty(&f.ret),
                f.name,
                args.join(", ")
            );
        }
        s + "}\n"
    }
}
