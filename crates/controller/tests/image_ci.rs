//! The build cluster path of the package and image workflows (image/ci/aws-build.sh).
use std::{fs, path::PathBuf};

fn read(p: &str) -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    fs::read_to_string(root.join(p)).unwrap_or_else(|e| panic!("{p}: {e}"))
}

/// The step bodies of a workflow, split at `- name:`.
fn steps(wf: &str) -> Vec<&str> {
    wf.split("      - name: ").skip(1).collect()
}

#[test]
fn the_signing_key_never_goes_to_the_cluster() {
    for wf in [
        ".github/workflows/packages.yml",
        ".github/workflows/iso.yml",
    ] {
        let w = read(wf);
        assert!(
            w.contains("id-token: write"),
            "{wf}: OIDC, no stored AWS keys"
        );
        for s in steps(&w).iter().filter(|s| s.contains("aws-build.sh")) {
            assert!(
                !s.contains("GPG_KEY") && !s.contains("RELEASE_KEY"),
                "{wf}: key in cluster step:\n{s}"
            );
        }
    }
    let script = read("image/ci/aws-build.sh");
    assert!(!script.contains("GPG") && !script.contains("SIGN_KEY"));
}

#[test]
fn both_workflows_still_build_without_the_cluster() {
    for wf in [
        ".github/workflows/packages.yml",
        ".github/workflows/iso.yml",
    ] {
        let w = read(wf);
        let local = steps(&w)
            .into_iter()
            .find(|s| {
                s.starts_with("Build in an archlinux container")
                    || s.starts_with("mkarchiso in an archlinux container")
            })
            .unwrap_or_else(|| panic!("{wf}: in-runner build step"));
        assert!(
            local.contains("vars.AWS_ROLE_ARN == ''"),
            "{wf}: fallback when no cluster"
        );
    }
}

#[test]
fn cluster_packages_are_signed_before_publishing() {
    let w = read(".github/workflows/packages.yml");
    let at = |name: &str| w.find(name).unwrap_or_else(|| panic!("step {name}"));
    assert!(at("Build on the build cluster") < at("Sign and index"));
    assert!(at("Sign and index") < at("Publish the repo"));
}
