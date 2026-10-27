//! General: the way into About, Software Update, Storage, Login Items, Language & Region, Date &
//! Time and Sharing (each a subpage with a back button).
use super::*;

/// (icon, title, subtitle) per row; rows of a group share a card.
const GROUPS: [&[(&str, &str, &str)]; 4] = [
    &[
        (
            "help-about-symbolic",
            "About",
            "Name, kernel, processor and memory",
        ),
        (
            "software-update-available-symbolic",
            "Software Update",
            "Updates with a restore point first",
        ),
        (
            "drive-harddisk-symbolic",
            "Storage",
            "What uses your disk, and restore points",
        ),
    ],
    &[(
        "view-app-grid-symbolic",
        "Login Items",
        "Apps that open when you log in",
    )],
    &[
        (
            "preferences-desktop-locale-symbolic",
            "Language & Region",
            "Language of menus and formats",
        ),
        (
            "preferences-system-time-symbolic",
            "Date & Time",
            "Time zone and network time",
        ),
    ],
    &[(
        "folder-publicshare-symbolic",
        "Sharing",
        "Computer name and remote login",
    )],
];

pub(super) fn page(e: &Rc<Env>) -> gtk::Box {
    let p = w::page("General");
    p.append(&w::footnote(
        "Manage your computer’s overall setup: updates, storage, language, time and sharing.",
    ));
    for g in GROUPS {
        let rows: Vec<gtk::Widget> = g
            .iter()
            .map(|(icon, title, sub)| {
                let b = form::nav_row(icon, title, sub);
                let open = e.opener(title);
                b.connect_clicked(move |_| open());
                b.upcast()
            })
            .collect();
        p.append(&w::section("", &rows));
    }
    p
}

/// A General subpage: back to General, large title.
pub(super) fn subpage(e: &Rc<Env>, title: &str) -> gtk::Box {
    form::subpage(title, e.opener("General"))
}
