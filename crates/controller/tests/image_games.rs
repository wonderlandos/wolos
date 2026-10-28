//! The image ships what the Steam client checks before it starts, a Steam launcher that runs
//! `wol games steam`, and a handler that opens Windows programs on Wine.
use controller::cli::parse;
use controller::games::STEAM_ICON;
use std::path::PathBuf;

fn repo(p: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(p)
}

fn read(p: &str) -> String {
    std::fs::read_to_string(repo(p)).unwrap_or_else(|e| panic!("{p}: {e}"))
}

fn key<'a>(desktop: &'a str, k: &str) -> &'a str {
    desktop
        .lines()
        .find_map(|l| l.strip_prefix(k)?.strip_prefix('='))
        .unwrap_or_else(|| panic!("no {k}="))
}

const APPS: &str = "desktop/overlay/usr/share/applications";

#[test]
fn steam_and_wine_are_native_packages() {
    let p = read("desktop/packages.txt");
    for want in [
        "steam",
        "wine",
        "lib32-mesa",
        "lib32-vulkan-intel",
        "lib32-vulkan-radeon",
    ] {
        assert!(
            p.lines().any(|l| l.split_whitespace().next() == Some(want)),
            "{want}"
        );
    }
    assert!(read("image/arch/pacman.conf").contains("\n[multilib]\n"));
}

#[test]
fn steam_opens_through_wol_games_steam() {
    let d = read(&format!("{APPS}/steam.desktop"));
    assert_eq!(key(&d, "Exec"), "wol games steam %u");
    let inv = parse(&["games", "steam", "steam://rungameid/620"]).unwrap();
    assert_eq!(inv.action, "games.steam");
    assert_eq!(inv.get("url"), Some("steam://rungameid/620"));
    assert!(key(&d, "MimeType").contains("x-scheme-handler/steam;"));
    // The App Store lists Steam as installed by this file (wol_core::store, desktop "steam").
    assert_eq!(wol_core::store::get("steam").unwrap().desktop, "steam");
    let icon = key(&d, "Icon");
    assert_eq!(icon, STEAM_ICON);
    let name = icon.rsplit('/').next().unwrap();
    assert!(repo(&format!("design/icons/store/{name}")).exists());
    assert!(
        read("image/repo/wolos-system/PKGBUILD").contains("$pkgdir/usr/share/wolos/store-icons")
    );
}

#[test]
fn windows_programs_open_on_wine_proton() {
    let d = read(&format!("{APPS}/org.wolos.windows-app.desktop"));
    assert_eq!(key(&d, "Exec"), "wol-run --exe %f");
    assert_eq!(key(&d, "NoDisplay"), "true");
    let mime = read(&format!("{APPS}/mimeapps.list"));
    for t in [
        "application/x-ms-dos-executable",
        "application/x-msdownload",
        "application/vnd.microsoft.portable-executable",
        "application/x-msi",
    ] {
        assert!(key(&d, "MimeType").contains(&format!("{t};")), "{t}");
        let line = format!("{t}=org.wolos.windows-app.desktop;");
        assert!(
            mime.lines().any(|l| l == line),
            "mimeapps.list lacks {line}"
        );
    }
    assert!(
        mime.lines()
            .any(|l| l == "x-scheme-handler/steam=steam.desktop;")
    );
}
