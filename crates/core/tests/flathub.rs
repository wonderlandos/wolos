// SPDX-License-Identifier: BSD-2-Clause
//! Flathub as a source of the App Store with WolOS's curation on top (docs/modules/flathub.md,
//! IMAGE-F). Three standings, consistent with the hub's records (hub repo docs/adr, not in this tree yet: who made it, and what
//! WolOS did): Picked (in `flathub::CURATED`: chosen by WolOS, not an audit), Verified (Flathub's
//! own proof that the developer owns the app id), Community (neither; the consent sheet still asks).
use wol_core::flathub::{self, Standing};
use wol_core::flatpak::{self, Hit};
use wol_core::{appstream, store};

fn hit(id: &str) -> Hit {
    Hit {
        id: id.into(),
        name: id.rsplit('.').next().unwrap().into(),
        summary: "x".into(),
    }
}

#[test]
fn the_curated_list_is_well_formed_and_never_repeats_a_native_app() {
    let mut seen = std::collections::HashSet::new();
    assert!(flathub::CURATED.len() >= 20, "a list worth opening");
    for c in flathub::CURATED {
        assert!(flatpak::is_app_id(c.id), "{}", c.id);
        assert!(seen.insert(c.id), "{} twice", c.id);
        assert!(!c.name.is_empty());
        // One line, sentence case, no full stop (the Store catalog's rule).
        assert!(!c.summary.ends_with('.'), "{}: {}", c.id, c.summary);
        assert!(c.summary.chars().next().unwrap().is_uppercase(), "{}", c.id);
        // One app, one tile: a catalog app or a native package of the same name wins over Flathub.
        let guess = flatpak::native_guess(&hit(c.id));
        assert!(
            store::CATALOG
                .iter()
                .all(|a| a.id != guess && !a.packages.contains(&guess.as_str())),
            "{} is in the native catalog as {guess}",
            c.id
        );
    }
}

#[test]
fn verified_ids_come_from_the_local_appstream() {
    let xml = r#"<components origin="flathub">
<component type="desktop-application"><id>org.a.A</id><name>A</name>
<custom><value key="flathub::verification::verified">true</value><value key="flathub::verification::method">website</value></custom></component>
<component type="desktop-application"><id>org.b.B</id><name>B</name><custom><value key="flathub::verification::verified">false</value></custom></component>
<component type="desktop-application"><id>org.c.C</id><name>C</name></component>
<component type="desktop-application"><id>org.d.D</id><custom><value key="flathub::verification::verified">true</value></custom></component>
</components>"#;
    let v = appstream::verified_ids(xml);
    let mut ids: Vec<&str> = v.iter().map(String::as_str).collect();
    ids.sort();
    assert_eq!(ids, ["org.a.A", "org.d.D"]);
    assert!(appstream::verified_ids("").is_empty());
}

#[test]
fn standing_is_picked_then_verified_then_community() {
    let verified: std::collections::HashSet<String> =
        ["org.v.V".to_owned(), "com.discordapp.Discord".to_owned()].into();
    assert_eq!(
        flathub::standing("com.discordapp.Discord", &verified),
        Standing::Picked { verified: true }
    );
    assert_eq!(
        flathub::standing("com.spotify.Client", &verified),
        Standing::Picked { verified: false }
    );
    assert_eq!(flathub::standing("org.v.V", &verified), Standing::Verified);
    assert_eq!(flathub::standing("org.x.X", &verified), Standing::Community);
}

#[test]
fn search_hits_are_ranked_by_standing_and_keep_their_order_within_one() {
    let verified: std::collections::HashSet<String> =
        ["org.v.V1".to_owned(), "org.v.V2".to_owned()].into();
    let hits = vec![
        hit("org.c.C1"),
        hit("org.v.V1"),
        hit("com.slack.Slack"),
        hit("org.c.C2"),
        hit("org.v.V2"),
        hit("com.bitwarden.desktop"),
    ];
    let ids: Vec<String> = flathub::rank(hits, &verified)
        .into_iter()
        .map(|h| h.id)
        .collect();
    assert_eq!(
        ids,
        [
            "com.slack.Slack",
            "com.bitwarden.desktop",
            "org.v.V1",
            "org.v.V2",
            "org.c.C1",
            "org.c.C2"
        ]
    );
}

#[test]
fn curated_hits_for_a_shelf_leave_out_what_a_native_package_covers() {
    let all = flathub::curated_hits(|_| false);
    assert_eq!(all.len(), flathub::CURATED.len());
    let some = flathub::curated_hits(|h| h.id == "com.slack.Slack");
    assert_eq!(some.len(), all.len() - 1);
    assert!(some.iter().all(|h| h.id != "com.slack.Slack"));
}
