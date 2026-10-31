// SPDX-License-Identifier: GPL-2.0-or-later
//! `include/wol.h` read into a small model, and the Crystal and .NET declarations written from it
//! (ADR 0020). The header is the one source: a new function, callback or constant in `wol.h` changes
//! what this writes, and `tests/bindings.rs` fails until the checked-in binding files are regenerated
//! (`WOL_BLESS=1 cargo test -p wol-abi --test bindings`). Swift needs no generated file: its module map
//! imports the header itself.
#![allow(dead_code)]

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
    /// `#define WOL_X 3`, in order.
    pub consts: Vec<(String, String)>,
    /// `typedef struct wol_x wol_x;`
    pub opaque: Vec<String>,
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
    for stmt in code.split(';') {
        let s = stmt.split_whitespace().collect::<Vec<_>>().join(" ");
        if s.is_empty() {
            continue;
        }
        if let Some(t) = s.strip_prefix("typedef ") {
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
            h.funcs.push(func(&s));
        }
    }
    h
}

fn camel(name: &str) -> String {
    name.trim_start_matches("wol_")
        .split('_')
        .map(|w| w[..1].to_uppercase() + &w[1..])
        .collect()
}

impl Header {
    fn is_alias(&self, n: &str) -> bool {
        self.aliases.iter().any(|(a, _)| a == n)
    }
    fn is_opaque(&self, n: &str) -> bool {
        self.opaque.iter().any(|o| o == n)
    }
    fn is_callback(&self, n: &str) -> bool {
        self.callbacks.iter().any(|c| c.name == n)
    }

    fn crystal_ty(&self, t: &Ty) -> String {
        let b = t.base.as_str();
        let (base, ptrs) = match b {
            "void" if t.ptrs == 0 => return "Void".into(),
            "void" => ("Void".to_string(), t.ptrs),
            "char" => ("UInt8".into(), t.ptrs),
            "size_t" => ("LibC::SizeT".into(), t.ptrs),
            "int32_t" | "int" => ("Int32".into(), t.ptrs),
            "uint32_t" => ("UInt32".into(), t.ptrs),
            "double" => ("Float64".into(), t.ptrs),
            n if self.is_opaque(n) || self.is_alias(n) || self.is_callback(n) => (camel(n), t.ptrs),
            n => panic!("no Crystal type for {n}"),
        };
        format!("{base}{}", "*".repeat(ptrs))
    }

    /// `lib LibWol` for Crystal.
    pub fn crystal(&self) -> String {
        let kw = ["out", "in", "end", "type", "self", "begin", "alias"];
        let mut s = String::from(
            "# SPDX-License-Identifier: GPL-2.0-or-later\n\
             # Generated from include/wol.h by crates/wol-abi/tests/bindings.rs; do not edit.\n\
             # Regenerate: WOL_BLESS=1 cargo test -p wol-abi --test bindings\n\
             @[Link(\"wol\")]\nlib LibWol\n",
        );
        for (n, v) in &self.consts {
            s += &format!("  {n} = {v}\n");
        }
        s.push('\n');
        for (n, t) in &self.aliases {
            s += &format!("  alias {} = {}\n", camel(n), self.crystal_ty(&ty(t)));
        }
        s += "  # Handles are opaque: the field exists only because a Crystal struct cannot be empty.\n";
        for n in &self.opaque {
            s += &format!("  struct {}; opaque : UInt8; end\n", camel(n));
        }
        s.push('\n');
        for c in &self.callbacks {
            let args: Vec<String> = c.params.iter().map(|(_, t)| self.crystal_ty(t)).collect();
            s += &format!(
                "  alias {} = {}-> {}\n",
                camel(&c.name),
                if args.is_empty() {
                    String::new()
                } else {
                    args.join(", ") + " "
                },
                self.crystal_ty(&c.ret)
            );
        }
        s.push('\n');
        for f in &self.funcs {
            let args: Vec<String> = f
                .params
                .iter()
                .map(|(n, t)| {
                    let n = if kw.contains(&n.as_str()) {
                        format!("{n}_")
                    } else {
                        n.clone()
                    };
                    format!("{n} : {}", self.crystal_ty(t))
                })
                .collect();
            let ret = if f.ret == ty("void") {
                String::new()
            } else {
                format!(" : {}", self.crystal_ty(&f.ret))
            };
            s += &format!("  fun {}({}){ret}\n", f.name, args.join(", "));
        }
        s + "end\n"
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
            "out", "string", "event", "ref", "in", "params", "object", "base", "lock",
        ];
        let id = |n: &str| {
            if kw.contains(&n) {
                format!("@{n}")
            } else {
                n.to_string()
            }
        };
        let mut s = String::from(
            "// SPDX-License-Identifier: GPL-2.0-or-later\n\
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
