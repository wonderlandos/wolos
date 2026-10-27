//! App Store install page for a double-clicked `.wol` (ADR 0011): signed, unsigned and damaged
//! files, read through the real `verify`, wide and narrow.
use super::Case;
use super::wol_file;
use gtk::prelude::*;
use shell::ui::store::{self, PackageFile};
use std::rc::Rc;

pub fn cases() -> Vec<Case> {
    let mut v = Vec::new();
    for (sw, sh, tag) in [(900, 600, "wide"), (480, 360, "narrow")] {
        for kind in ["signed", "unsigned", "damaged"] {
            v.push(Case {
                name: format!("store-package-{kind}-{tag}"),
                size: (sw, sh),
                class: "preview-app",
                wallpaper: false,
                build: Box::new(move || {
                    let d = wol_file::dir("golden");
                    let p = match kind {
                        "signed" => {
                            wol_file::write(&d, "Shredder.wol", wol_file::SHREDDER, None, true)
                        }
                        "unsigned" => wol_file::write(&d, "Wike.wol", wol_file::WIKE, None, false),
                        _ => wol_file::tampered(&wol_file::write(
                            &d,
                            "Shredder.wol",
                            wol_file::SHREDDER,
                            None,
                            true,
                        )),
                    };
                    let s = store::content(
                        Rc::new(shell::system::Fake),
                        store::Env {
                            data_dirs: vec![],
                            pkg_search: false,
                        },
                    );
                    s.show_package(PackageFile::read_with(
                        p.to_str().unwrap(),
                        &[wol_file::trusted()],
                        &d.join("records"),
                    ));
                    s.aw.root.clone().upcast()
                }),
            });
        }
    }
    v
}
