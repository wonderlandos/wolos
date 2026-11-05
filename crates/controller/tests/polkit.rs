// SPDX-License-Identifier: GPL-2.0-or-later
//! Contract for files/usr/share/polkit-1/rules.d/10-wolos.rules (audit A16/B6): WolOS actions are
//! never granted through polkit (they go through wol-admind and wold), nothing is ever granted
//! without authentication, and a subject that is not the active local session or an administrator
//! is refused. polkit reads rules.d files in name order and the first non-NOT_HANDLED answer wins,
//! so the file sorts before the distribution's 50-default.rules.
#[path = "support/siblings.rs"]
mod siblings;

const RULES: &str = "files/usr/share/polkit-1/rules.d/10-wolos.rules";

fn rules() -> String {
    std::fs::read_to_string(siblings::root().join(RULES)).unwrap_or_else(|e| panic!("{RULES}: {e}"))
}

/// The statements of the rule body, comments dropped, whitespace squeezed.
fn code() -> String {
    rules()
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with("//") && !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn wolos_actions_are_refused_first() {
    let c = code();
    let deny = c
        .find(r#"if (action.id.indexOf("org.wolos.") === 0) { return polkit.Result.NO; }"#)
        .expect("org.wolos.* -> NO");
    let wheel = c
        .find("isInGroup(\"wheel\")")
        .expect("administrators keep upstream behaviour");
    assert!(
        deny < wheel,
        "the WolOS refusal comes before any pass-through"
    );
}

#[test]
fn nothing_is_granted_without_authentication() {
    let c = code();
    for never in ["Result.YES", "AUTH_SELF_KEEP", "AUTH_ADMIN_KEEP", "spawn("] {
        assert!(!c.contains(never), "{RULES} must not use {never}");
    }
    assert_eq!(c.matches("polkit.addRule(").count(), 1, "one rule");
}

#[test]
fn remote_and_background_subjects_are_refused() {
    let c = code();
    assert!(
        c.contains("if (!subject.local || !subject.active) { return polkit.Result.NO; }"),
        "{c}"
    );
    assert!(
        c.ends_with("return polkit.Result.NOT_HANDLED; });"),
        "falls through to upstream"
    );
}

/// Audit S17: root daemons have no session (local/active false); they must not fall into the
/// remote refusal, but they get no WolOS action either.
#[test]
fn root_subjects_without_a_session_keep_upstream_behaviour() {
    let c = code();
    let root = c
        .find(r#"if (subject.user === "root") { return polkit.Result.NOT_HANDLED; }"#)
        .expect("uid 0 passes through");
    let deny = c.find("org.wolos.").expect("org.wolos.* refusal");
    let remote = c
        .find("if (!subject.local || !subject.active)")
        .expect("remote refusal");
    assert!(deny < root, "root gets no org.wolos.* action either");
    assert!(
        root < remote,
        "root is let through before the session check"
    );
}

#[test]
fn the_file_sorts_before_the_distribution_defaults() {
    let name = RULES.rsplit('/').next().unwrap_or("");
    assert!(name.as_bytes() < b"50-default.rules".as_slice(), "{name}");
    assert!(rules().starts_with("// SPDX-License-Identifier: GPL-2.0-or-later\n"));
}

/// QA-FILES-23: a standard user who clicks an internal disk in Files is refused, not shown an
/// administrator password dialog; administrators keep the prompt (the wheel pass-through comes
/// first). The installer's key partition is not listed at all.
#[test]
fn a_standard_user_cannot_mount_an_internal_disk() {
    let c = code();
    let wheel = c.find("isInGroup(\"wheel\")").unwrap();
    for id in [
        "filesystem-mount-system",
        "filesystem-fstab",
        "encrypted-unlock-system",
    ] {
        let at = c
            .find(&format!("\"org.freedesktop.udisks2.{id}\""))
            .unwrap_or_else(|| panic!("{id} is not refused"));
        assert!(at > wheel, "{id}: administrators keep upstream behaviour");
    }
    assert!(
        c.contains(
            "\"org.freedesktop.udisks2.encrypted-unlock-system\" ) { return polkit.Result.NO; }"
        ),
        "{c}"
    );
    let udev = std::fs::read_to_string(
        siblings::root().join("files/usr/lib/udev/rules.d/80-wolos-udisks.rules"),
    )
    .unwrap();
    assert!(
        udev.contains(r#"ENV{ID_FS_LABEL}=="WOLOSKEY", ENV{UDISKS_IGNORE}="1""#),
        "{udev}"
    );
}
