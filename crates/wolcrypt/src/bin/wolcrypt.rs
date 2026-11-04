// SPDX-License-Identifier: BSD-2-Clause
//! `wolcrypt`: checks published WolOS files with nothing but this crate.
//!
//! ```text
//! wolcrypt verify-detached FILE FILE.wsig WITNESS.wolcert ROOT_A.wolcert ROOT_B.wolcert
//! wolcrypt sha512 FILE
//! ```
//! Prints the signature line and exits 0 when the witness's signature verifies; 1 with a reason
//! otherwise; 2 on a usage error.
#![forbid(unsafe_code)]

fn read(p: &str) -> Result<Vec<u8>, String> {
    std::fs::read(p).map_err(|e| format!("{p}: {e}"))
}

fn run(args: &[String]) -> Result<String, (i32, String)> {
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    match a[..] {
        ["verify-detached", f, s, w, ra, rb] => {
            let r = |p| read(p).map_err(|e| (1, e));
            let (file, sig, wit, a, b) = (r(f)?, r(s)?, r(w)?, r(ra)?, r(rb)?);
            wolcrypt::detached::verify(&file, &sig, &wit, [&a, &b])
                .map_err(|e| (1, format!("not verified: {}", e.as_str())))
        }
        ["sha512", f] => Ok(wolcrypt::hash::hex(&wolcrypt::hash::sha512(
            &read(f).map_err(|e| (1, e))?,
        ))),
        _ => Err((
            2,
            "usage: wolcrypt verify-detached FILE FILE.wsig WITNESS.wolcert ROOT_A.wolcert ROOT_B.wolcert | wolcrypt sha512 FILE".into(),
        )),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(line) => println!("{line}"),
        Err((code, m)) => {
            eprintln!("wolcrypt: {m}");
            std::process::exit(code);
        }
    }
}
