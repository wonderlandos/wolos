//! Every catalog app ships its official icon (design/icons/store/<id>.svg, < 100 KB, licensed).
use shell::kit;
use wol_core::store::CATALOG;

#[test]
fn every_catalog_app_has_a_real_icon() {
    let licenses = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../design/icons/store/LICENSES.md"
    ))
    .unwrap();
    for a in CATALOG {
        let p = kit::store_icon_file(a.id).unwrap_or_else(|| panic!("no icon for {}", a.id));
        let len = std::fs::metadata(&p).unwrap().len();
        assert!(len < 100 * 1024, "{} is {len} bytes", a.id);
        let b = gtk::gdk_pixbuf::Pixbuf::from_file_at_scale(&p, 64, 64, true)
            .unwrap_or_else(|e| panic!("{} does not render: {e}", a.id));
        assert_eq!((b.width().max(b.height())), 64, "{}", a.id);
        assert!(
            licenses.contains(&format!("| {} |", a.id)),
            "{} license",
            a.id
        );
    }
}
