// SPDX-License-Identifier: GPL-2.0-or-later
//! `wol shard list|add|update|check` (docs/modules/shards.md, docs/RULES-crystal.md RCR-040 to
//! RCR-044): Crystal shards from the signed WolOS shard list, pinned by full commit in a project's
//! `shard.yml` with a `shard.lock` that `shards install --frozen --local` accepts offline. The work
//! is `wopkg shard ...` on libwopkg (`wopkg_shard_*`, crates/wopkg/src/index/shards.rs): this
//! module maps the `wol` words to that argv and passes the exit status through (off a terminal,
//! the JSON output too). Nothing here needs root; `add` and `update` change only the project's
//! two files and the person's shards cache.
use crate::cli::Invocation;
use crate::run::{Env, fail};
use shared::errors::ErrorCode;

/// The program that does the work (on PATH in every image that has `wol`).
pub const WOPKG: &str = "wopkg";

/// A project folder as `wopkg shard` reads it: a word with a `/` (shard names never hold one).
fn folder(d: &str) -> String {
    if d.contains('/') || d == "." || d == ".." {
        d.to_owned()
    } else {
        format!("./{d}")
    }
}

/// The `wopkg` argv for one `wol shard` invocation (pure; tests read it).
pub fn argv(inv: &Invocation) -> Result<Vec<String>, String> {
    let get = |k: &str| inv.get(k).filter(|v| !v.is_empty());
    let mut a: Vec<String> = vec![WOPKG.into(), "shard".into()];
    match inv.action {
        "shard.list" => {
            a.push("list".into());
            a.extend(inv.all("term").into_iter().map(str::to_owned));
        }
        "shard.add" => {
            let name = get("shard").ok_or("Name the shard to add: wol shard add <shard> [dir]")?;
            if name.contains('/') {
                return Err(format!("{name} is a folder, not a shard name."));
            }
            a.extend(["add".into(), name.to_owned()]);
            a.extend(get("dir").map(folder));
        }
        "shard.update" => {
            a.push("update".into());
            // `wol shard update ./app`: one word that is a folder is the folder.
            match (get("shard"), get("dir")) {
                (Some(s), None) if s.contains('/') || s == "." || s == ".." => a.push(s.to_owned()),
                (s, d) => {
                    a.extend(s.map(str::to_owned));
                    a.extend(d.map(folder));
                }
            }
        }
        "shard.check" => {
            a.push("check".into());
            a.extend(get("dir").map(folder));
        }
        other => return Err(format!("no handler for {other}")),
    }
    for f in ["json", "local"] {
        if inv.flag(f) {
            a.push(format!("--{f}"));
        }
    }
    Ok(a)
}

/// On a terminal, `wopkg` prints to it. Otherwise (a pipe, a script, `wol mcp`, or `--json`) the
/// output is wopkg's JSON document, captured and printed through `env` so a helper gets it on
/// the protocol, and the exit status still comes back (1 when `check` found something).
pub fn execute(inv: &Invocation, env: &mut dyn Env) -> i32 {
    let mut a = match argv(inv) {
        Ok(a) => a,
        Err(m) => return fail(env, ErrorCode::Usage, &m),
    };
    let json = inv.flag("json") || !env.stdout_tty();
    if json && !a.iter().any(|w| w == "--json") {
        a.push("--json".into());
    }
    let ran = if json {
        env.exec_output(&a).map(|(code, text)| {
            env.out(&text);
            code
        })
    } else {
        env.exec(&a)
    };
    match ran {
        Ok(code) => code,
        Err(_) => fail(
            env,
            ErrorCode::Unavailable,
            "wopkg is not installed, so shards cannot be managed here.",
        ),
    }
}
