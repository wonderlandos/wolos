//! Job definitions (`*.json` in the jobs dir) and pid files for crash-restart cleanup.
use shared::jobs::JobSpec;
use std::path::{Path, PathBuf};

pub const PID_DIR: &str = "/var/run/wold";

/// Valid jobs sorted by label, plus one error line per skipped file. Duplicate labels: first wins.
pub fn load(dir: &Path) -> (Vec<JobSpec>, Vec<String>) {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|d| {
            d.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "json"))
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    let (mut jobs, mut errors) = (Vec::<JobSpec>::new(), Vec::new());
    for f in files {
        match std::fs::read_to_string(&f)
            .map_err(|e| e.to_string())
            .and_then(|t| JobSpec::parse(&t))
        {
            Ok(j) if jobs.iter().any(|o| o.label == j.label) => {
                errors.push(format!("{}: duplicate label {}", f.display(), j.label))
            }
            Ok(j) => jobs.push(j),
            Err(e) => errors.push(format!("{}: {e}", f.display())),
        }
    }
    jobs.sort_by(|a, b| a.label.cmp(&b.label));
    (jobs, errors)
}

/// Pid file path; labels are file names, so path separators are rejected.
pub fn pid_file(dir: &Path, label: &str) -> Option<PathBuf> {
    (!label.contains('/') && !label.starts_with('.')).then(|| dir.join(format!("{label}.pid")))
}

pub fn read_pid(path: &Path) -> Option<i32> {
    std::fs::read_to_string(path)
        .ok()?
        .trim()
        .parse()
        .ok()
        .filter(|p: &i32| *p > 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_skips_bad_and_duplicate_files() {
        let d = std::env::temp_dir().join(format!("wold-jobs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        for (name, body) in [
            ("1.json", r#"{"label":"b","program":["/bin/b"]}"#),
            ("2.json", r#"{"label":"a","program":["/bin/a"]}"#),
            ("3.json", r#"{"label":"a","program":["/bin/dup"]}"#),
            ("4.json", "nope"),
            ("5.txt", "ignored"),
        ] {
            std::fs::write(d.join(name), body).unwrap();
        }
        let (jobs, errors) = load(&d);
        assert_eq!(
            jobs.iter().map(|j| j.label.as_str()).collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert_eq!(jobs[0].program, ["/bin/a"]);
        assert_eq!(errors.len(), 2);
        let pf = pid_file(&d, "a").unwrap();
        std::fs::write(&pf, "1234\n").unwrap();
        assert_eq!(read_pid(&pf), Some(1234));
        std::fs::write(&pf, "1").unwrap();
        assert_eq!(read_pid(&pf), None, "never signal init");
        assert_eq!(pid_file(&d, "../x"), None);
        std::fs::remove_dir_all(&d).unwrap();
        assert_eq!(load(&d).0, vec![]);
    }
}
