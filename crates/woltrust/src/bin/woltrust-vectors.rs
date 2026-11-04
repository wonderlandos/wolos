// SPDX-License-Identifier: BSD-2-Clause
//! `woltrust-vectors <dir>` writes pki.json and one JSON file per vector into `<dir>`, and the
//! sealed-message vectors (keys.json and one file each) into `<dir>/seal` and the certificate
//! request vectors into `<dir>/request` and the file certificate vectors into `<dir>/filecert`;
//! `woltrust-vectors --check <dir>` regenerates them and fails if any byte differs. A development
//! tool over the TEST authority; it is not shipped and holds no production key.
#![forbid(unsafe_code)]
use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (check, dir) = match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["--check", d] => (true, d),
        [d] => (false, d),
        _ => {
            eprintln!("usage: woltrust-vectors [--check] <dir>");
            std::process::exit(2);
        }
    };
    match run(check, Path::new(dir)) {
        Ok(n) => println!("{n} files {}", if check { "match" } else { "written" }),
        Err(e) => {
            eprintln!("woltrust-vectors: {e}");
            std::process::exit(1);
        }
    }
}

fn run(check: bool, dir: &Path) -> Result<usize, String> {
    let (pki, vectors) = woltrust::vectors::generate()?;
    let mut files = vec![("pki.json".to_owned(), woltrust::vectors::to_text(&pki))];
    for v in &vectors {
        files.push((format!("{}.json", v.name), woltrust::vectors::to_text(v)));
    }
    let (keys, sealed) = woltrust::seal_vectors::generate()?;
    files.push((
        "seal/keys.json".to_owned(),
        woltrust::vectors::to_text(&keys),
    ));
    for v in &sealed {
        files.push((
            format!("seal/{}.json", v.name),
            woltrust::vectors::to_text(v),
        ));
    }
    for v in &woltrust::filecert_vectors::generate()? {
        files.push((
            format!("filecert/{}.json", v.name),
            woltrust::vectors::to_text(v),
        ));
    }
    for v in &woltrust::request_vectors::generate()? {
        files.push((
            format!("request/{}.json", v.name),
            woltrust::vectors::to_text(v),
        ));
    }
    for (name, text) in &files {
        let path = dir.join(name);
        if check {
            let have =
                std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            if &have != text {
                return Err(format!("{} differs", path.display()));
            }
        } else {
            for sub in ["seal", "request", "filecert"] {
                std::fs::create_dir_all(dir.join(sub)).map_err(|e| e.to_string())?;
            }
            std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
        }
    }
    Ok(files.len())
}
