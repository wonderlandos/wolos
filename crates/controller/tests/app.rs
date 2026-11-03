// SPDX-License-Identifier: GPL-2.0-or-later
//! `wol app new|run|check` (crates/controller/src/app.rs): the four templates of
//! wolos-crystal/templates render with the I7 placeholders filled and nothing left over, a new
//! project never lands on an existing folder, `run --watch` adds WOL_APP_WATCH=1 and nothing else,
//! and `check` reports the RCR findings of a project and type-checks a clean one with `make check`.
//! Everything goes through FakeEnv: no file is written and no program starts.
use controller::app::{self, Names};
use controller::cli::parse;
use controller::run::{FakeEnv, execute};
use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every file of the repo's templates folder, as FakeEnv files under `/t`.
fn template_files() -> Vec<(PathBuf, String)> {
    fn go(dir: &Path, rel: &Path, out: &mut Vec<(PathBuf, String)>) {
        let mut entries: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let p = e.path();
            let r = rel.join(e.file_name());
            if p.is_dir() {
                go(&p, &r, out);
            } else {
                out.push((
                    Path::new("/t").join(&r),
                    std::fs::read_to_string(&p).unwrap(),
                ));
            }
        }
    }
    let mut out = Vec::new();
    go(
        &repo().join("wolos-crystal/templates"),
        Path::new(""),
        &mut out,
    );
    out
}

fn env_with_templates() -> FakeEnv {
    FakeEnv {
        files: template_files(),
        vars: vec![("WOLKIT_TEMPLATES".into(), "/t".into())],
        ..Default::default()
    }
}

fn run(env: &mut FakeEnv, argv: &[&str]) -> i32 {
    let inv = parse(argv).unwrap_or_else(|e| panic!("{argv:?}: {e}"));
    execute(&inv, env)
}

/// The project `wol app new <name> --template <t>` writes: (relative path, text).
fn rendered(name: &str, template: &str) -> Vec<(String, String)> {
    let mut e = env_with_templates();
    assert_eq!(
        run(&mut e, &["app", "new", name, "--template", template]),
        0,
        "{template}: {}",
        e.err
    );
    let short = name.rsplit('.').next().unwrap();
    e.written
        .iter()
        .map(|(p, t)| {
            let rel = p
                .strip_prefix(short)
                .unwrap_or_else(|_| panic!("{p:?} outside {short}"));
            (rel.to_string_lossy().into_owned(), t.clone())
        })
        .collect()
}

#[test]
fn names_follow_i7() {
    assert_eq!(
        app::names("my-notes").unwrap(),
        Names {
            id: "org.wolos.mynotes".into(),
            name: "my-notes".into(),
            module: "MyNotes".into()
        }
    );
    let n = app::names("org.example.notes").unwrap();
    assert_eq!(
        (n.id.as_str(), n.name.as_str(), n.module.as_str()),
        ("org.example.notes", "notes", "Notes")
    );
    for bad in [
        "",
        "Notes",
        "1st",
        "a b",
        "../x",
        "x/y",
        "org..x",
        "org.example.",
        ".x",
        "a".repeat(33).as_str(),
    ] {
        assert!(app::names(bad).is_err(), "{bad:?} was accepted");
    }
    // A module that already exists in the standard library or WolOS would be reopened.
    for taken in ["file", "json", "spec", "wolkit", "cryswol", "wol"] {
        assert!(app::names(taken).is_err(), "{taken} was accepted");
    }
}

#[test]
fn the_four_templates_exist() {
    for t in app::TEMPLATES {
        let dir = repo().join("wolos-crystal/templates").join(t);
        for f in [
            "shard.yml.tmpl",
            "Makefile.tmpl",
            "README.md.tmpl",
            ".gitignore.tmpl",
        ] {
            assert!(dir.join(f).is_file(), "{t}: {f} is missing");
        }
    }
    assert_eq!(app::TEMPLATES, ["app", "cli", "tool", "ai"]);
}

#[test]
fn every_template_renders_with_nothing_left_over() {
    for t in app::TEMPLATES {
        let files = rendered("demo-tool", t);
        let names: Vec<&str> = files.iter().map(|f| f.0.as_str()).collect();
        for must in [
            "shard.yml",
            "Makefile",
            "README.md",
            ".gitignore",
            "src/main.cr",
        ] {
            assert!(names.contains(&must), "{t}: no {must} in {names:?}");
        }
        assert!(
            names
                .iter()
                .any(|n| n.starts_with("spec/") && n.ends_with("_spec.cr")),
            "{t}: no spec"
        );
        for (rel, text) in &files {
            assert!(!rel.ends_with(".tmpl"), "{t}: {rel} kept its suffix");
            assert!(
                !rel.contains("{{"),
                "{t}: {rel}: placeholder in the file name"
            );
            assert!(
                !text.contains("{{"),
                "{t}: {rel}: a placeholder is left:\n{text}"
            );
        }
        let yml = &files.iter().find(|f| f.0 == "shard.yml").unwrap().1;
        assert!(yml.contains("name: demo-tool\n"), "{t}: {yml}");
        assert!(
            yml.contains("  demo-tool:\n    main: src/main.cr"),
            "{t}: {yml}"
        );
    }
    // File names carry the name too (the cli template's src/<name>.cr).
    let cli = rendered("demo-tool", "cli");
    assert!(cli.iter().any(|f| f.0 == "src/demo-tool.cr"));
    assert!(cli.iter().any(|f| f.1.contains("module DemoTool")));
    let tool = rendered("org.example.lamp", "tool");
    assert!(
        tool.iter()
            .any(|f| f.1.contains(r#"APP_ID  = "org.example.lamp""#))
    );
}

#[test]
fn new_defaults_to_the_app_template_and_says_what_it_wrote() {
    let mut e = env_with_templates();
    assert_eq!(run(&mut e, &["app", "new", "counter"]), 0, "{}", e.err);
    assert!(
        e.written
            .iter()
            .any(|w| w.0 == Path::new("counter/src/views/main.cr"))
    );
    assert!(e.out.contains("counter/src/app.cr\n"), "{}", e.out);
    assert!(e.out.contains("from the app template"), "{}", e.out);
    let app_cr = &e
        .written
        .iter()
        .find(|w| w.0 == Path::new("counter/src/app.cr"))
        .unwrap()
        .1;
    assert!(app_cr.contains("class Counter < Wolkit::App"), "{app_cr}");
    // --json is the machine form.
    let mut e = env_with_templates();
    assert_eq!(
        run(&mut e, &["app", "new", "org.example.todo", "cli", "--json"]),
        0,
        "{}",
        e.err
    );
    let v: serde_json::Value = serde_json::from_str(&e.out).unwrap();
    assert_eq!(
        (v["id"].as_str(), v["template"].as_str(), v["dir"].as_str()),
        (Some("org.example.todo"), Some("cli"), Some("todo"))
    );
}

#[test]
fn new_refuses_what_it_cannot_do() {
    // An existing folder is never written into.
    let mut e = env_with_templates();
    e.files
        .push((PathBuf::from("taken/notes.txt"), "mine".into()));
    assert_ne!(run(&mut e, &["app", "new", "taken"]), 0);
    assert!(e.err.contains("exists already"), "{}", e.err);
    assert!(e.written.is_empty());
    // An unknown template, a missing template name, a bad name.
    for argv in [
        &["app", "new", "x", "--template", "game"][..],
        &["app", "new", "x", "--template"][..],
        &["app", "new", "Bad Name"][..],
    ] {
        let mut e = env_with_templates();
        assert_eq!(run(&mut e, argv), 64, "{argv:?}: {}", e.err);
        assert!(e.written.is_empty(), "{argv:?}");
    }
    // No templates installed: say how to get them.
    let mut e = FakeEnv::default();
    assert_ne!(run(&mut e, &["app", "new", "x"]), 0);
    assert!(e.err.contains("wol install wolos-crystal"), "{}", e.err);
}

#[test]
fn run_sets_wol_app_watch_and_nothing_else() {
    assert_eq!(
        app::run_argv(Path::new("."), false),
        ["make", "-C", ".", "run"]
    );
    assert_eq!(
        app::run_argv(Path::new("notes"), true),
        ["env", "WOL_APP_WATCH=1", "make", "-C", "notes", "run"]
    );
    let mut e = FakeEnv {
        files: vec![(PathBuf::from("notes/Makefile"), "run:\n".into())],
        ..Default::default()
    };
    assert_eq!(
        run(&mut e, &["app", "run", "notes", "--watch"]),
        0,
        "{}",
        e.err
    );
    assert_eq!(
        e.execs,
        [["env", "WOL_APP_WATCH=1", "make", "-C", "notes", "run"]]
    );
    // No Makefile: nothing runs.
    let mut e = FakeEnv::default();
    assert_ne!(run(&mut e, &["app", "run", "elsewhere"]), 0);
    assert!(e.execs.is_empty());
    assert!(e.err.contains("no Makefile"), "{}", e.err);
}

/// A FakeEnv whose files are `project` under the folder `p`.
fn project_env(project: &[(String, String)]) -> FakeEnv {
    FakeEnv {
        files: project
            .iter()
            .map(|(r, t)| (Path::new("p").join(r), t.clone()))
            .collect(),
        ..Default::default()
    }
}

#[test]
fn check_passes_every_template_and_then_type_checks() {
    for t in app::TEMPLATES {
        let files = rendered("demo", t);
        let e = project_env(&files);
        let found = app::check(&e, Path::new("p"));
        assert!(found.is_empty(), "{t}: {found:#?}");
        let mut e = project_env(&files);
        assert_eq!(
            run(&mut e, &["app", "check", "p"]),
            0,
            "{t}: {}{}",
            e.out,
            e.err
        );
        assert_eq!(e.execs, [["make", "-s", "-C", "p", "check"]], "{t}");
        assert!(
            e.out
                .contains("follow the WolOS rules, and the program type-checks"),
            "{}",
            e.out
        );
        // --rules reads the files only.
        let mut e = project_env(&files);
        assert_eq!(run(&mut e, &["app", "check", "p", "--rules"]), 0);
        assert!(e.execs.is_empty());
    }
}

#[test]
fn check_reports_findings_and_starts_nothing() {
    let mut files = rendered("demo", "app");
    // A public method with no doc comment (RCR-052), the collector turned off in an app that
    // needs it (RCR-051), a dependency pinned by version (RCR-040), a shell command line (RCR-020).
    files.push((
        "src/extra.cr".into(),
        "# SPDX-License-Identifier: BSD-2-Clause\n\
            # Extra: a file that breaks four rules on purpose, one per line below.\n\
            module Extra\n  def self.go\n    system(\"ls #{Dir.current}\")\n  end\nend\n"
            .into(),
    ));
    for f in files.iter_mut() {
        if f.0 == "Makefile" {
            f.1.push_str("\nfast:\n\tcrystal build -Dgc_none src/main.cr\n");
        }
        if f.0 == "shard.yml" {
            f.1.push_str("dependencies:\n  other:\n    git: https://example.org/other.git\n    version: ~> 1.0\n");
        }
    }
    let mut e = project_env(&files);
    assert_eq!(run(&mut e, &["app", "check", "p"]), 1, "{}", e.out);
    assert!(e.execs.is_empty(), "a project with findings is not built");
    for want in [
        "src/extra.cr:4: RCR-052",
        "src/extra.cr:5: RCR-020",
        "Makefile:",
        "RCR-051",
        "shard.yml:",
        "RCR-040",
    ] {
        assert!(e.out.contains(want), "{want} missing:\n{}", e.out);
    }
    let mut e = project_env(&files);
    assert_eq!(run(&mut e, &["app", "check", "p", "--json"]), 1);
    let v: serde_json::Value = serde_json::from_str(&e.out).unwrap();
    assert!(
        v["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["rule"] == "RCR-052"),
        "{v}"
    );
    // The collector rule does not apply to a project that needs no runtime (the cli template).
    let cli = rendered("demo", "cli");
    assert!(
        cli.iter()
            .any(|f| f.0 == "Makefile" && f.1.contains("-Dgc_none"))
    );
    assert!(app::check(&project_env(&cli), Path::new("p")).is_empty());
    // A folder with no shard.yml is not a project.
    let found = app::check(&FakeEnv::default(), Path::new("empty"));
    assert!(
        found
            .iter()
            .any(|p| p.rule == "RCR-040" && p.why.contains("no shard.yml"))
    );
}

#[test]
fn public_doc_scanner() {
    let ok = "# A.\n\
        class A\n  # Doc.\n  def a\n  end\n\n  # Doc.\n  @[AlwaysInline]\n  def self.b : Int32\n    1\n  end\n\n  \
        private def c\n  end\n\n  \
        protected def d\n  end\nend\n";
    assert!(
        app::scan_public_docs(ok).is_empty(),
        "{:?}",
        app::scan_public_docs(ok)
    );
    let bad = "class A\n  def a\n  end\n\n  abstract def b\n  x = \"def c\"\nend\n";
    let found = app::scan_public_docs(bad);
    assert_eq!(
        found.iter().map(|f| f.0).collect::<Vec<_>>(),
        [2, 5],
        "{found:?}"
    );
    assert!(found[0].1.contains("`a`"), "{found:?}");
    // A comment in Makefile syntax is not a build flag.
    assert!(app::scan_gc_none("# no -Dgc_none here\nall:\n").is_empty());
    assert_eq!(
        app::scan_gc_none("all:\n\tcrystal build -Dgc_none x.cr\n").len(),
        1
    );
}
