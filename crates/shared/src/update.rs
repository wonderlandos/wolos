// SPDX-License-Identifier: BSD-2-Clause
//! The one disk rule of the update check: `wol update check` and Settings > Software Update both
//! ask "is there room to write what the check found?" of the same place, with the same number.

/// Where the daily check keeps its findings and pacman its databases: the disk whose free space
/// decides whether a check could have been written.
pub const CHECK_DISK: &str = "/var";

/// Below this much free space on [`CHECK_DISK`] a refresh cannot be written (50 MiB).
pub const NO_ROOM_BYTES: u64 = 50 << 20;

/// `df -B1 --output=avail <CHECK_DISK>`.
pub const FREE_ARGV: [&str; 4] = ["df", "-B1", "--output=avail", CHECK_DISK];

/// Free bytes from [`FREE_ARGV`]'s output (a header line, then the number).
pub fn free_from_df(out: &str) -> Option<u64> {
    out.lines().nth(1)?.trim().parse().ok()
}

/// Whether `free` bytes on [`CHECK_DISK`] is too little for a check.
pub fn no_room(free: Option<u64>) -> bool {
    free.is_some_and(|f| f < NO_ROOM_BYTES)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_rule_for_wol_and_settings() {
        assert_eq!(free_from_df("Avail\n1234\n"), Some(1234));
        assert_eq!(free_from_df("Avail\n"), None);
        assert_eq!(free_from_df("df: /var: no such file"), None);
        assert!(no_room(Some(0)) && no_room(Some(NO_ROOM_BYTES - 1)));
        assert!(!no_room(Some(NO_ROOM_BYTES)) && !no_room(None));
        assert_eq!(FREE_ARGV.join(" "), "df -B1 --output=avail /var");
    }
}
