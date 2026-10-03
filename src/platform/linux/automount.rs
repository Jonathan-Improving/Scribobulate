//! Whether a directory is an automount point, read from this process's mount table.
//!
//! Linux's automounter (`autofs`, from the `autofs` daemon's maps or a systemd
//! `.automount` unit) mounts an `autofs` filesystem at each map's directory. Looking up a
//! name inside an INDIRECT map mounts what the name denotes: with `/net -hosts`,
//! `/net/<host>` asks `<host>` for its NFS exports. A DIRECT map's directory is itself
//! the trigger.
//!
//! **Read from `/proc/self/mountinfo`, never through the path.** `statfs` on the
//! directory would answer for an indirect map's root, but at a direct mount point that is
//! already mounted it reports the filesystem stacked on top, not `autofs` (MEASURED,
//! Ubuntu 22.04 / kernel 6.8: `stat -f` on `/proc/sys/fs/binfmt_misc` reads
//! `binfmt_misc` while the mount table lists an `autofs` mount at the same point). Whether
//! `statfs` on a direct point that is NOT yet mounted would mount it was not measured.
//! The mount table answers both cases without touching the path at all.
//!
//! Supplies the fact only. What to do about it is the image and link gates' decision
//! (`links::links_reach_foreign`).

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

/// Whether an `autofs` filesystem is mounted at exactly `dir`, so that looking up a name
/// inside it can mount something. `false` when the mount table cannot be read.
pub(crate) fn is_automount_directory(dir: &Path) -> bool {
    let Ok(table) = std::fs::read("/proc/self/mountinfo") else {
        return false;
    };
    autofs_at(&table, dir)
}

/// Whether the `mountinfo` text `table` lists an `autofs` mount whose mount point is `dir`.
fn autofs_at(table: &[u8], dir: &Path) -> bool {
    table.split(|&b| b == b'\n').any(|line| {
        // `<id> <parent> <maj:min> <root> <mount point> <options> [optional...] - <fstype> ...`
        let mut fields = line.split(|&b| b == b' ');
        let Some(point) = fields.nth(4) else {
            return false;
        };
        let fstype = fields.skip_while(|f| *f != b"-").nth(1);
        fstype == Some(b"autofs".as_slice())
            && Path::new(OsStr::from_bytes(&unescape(point))) == dir
    })
}

/// Undo `mountinfo`'s octal escapes (`\040` space, `\011` tab, `\012` newline, `\134`
/// backslash), so a mount point with a space in it compares equal to its path.
fn unescape(field: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(field.len());
    let mut i = 0;
    while i < field.len() {
        let octal = field
            .get(i + 1..i + 4)
            .filter(|d| field[i] == b'\\' && d.iter().all(|c| (b'0'..=b'7').contains(c)));
        if let Some(d) = octal {
            out.push(
                d.iter()
                    .fold(0u8, |acc, c| acc.wrapping_mul(8) + (c - b'0')),
            );
            i += 4;
        } else {
            out.push(field[i]);
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{autofs_at, is_automount_directory};
    use std::path::Path;

    const TABLE: &[u8] = b"\
22 1 8:1 / / rw,relatime shared:1 - ext4 /dev/sda1 rw
39 26 0:34 / /proc/sys/fs/binfmt_misc rw,relatime shared:14 - autofs systemd-1 rw,direct
41 39 0:40 / /proc/sys/fs/binfmt_misc rw,relatime shared:20 - binfmt_misc binfmt_misc rw
50 22 0:74 / /net rw,relatime shared:9 - autofs /etc/auto.net rw,indirect
51 22 0:75 / /mnt/my\\040maps rw,relatime - autofs /etc/auto.x rw,indirect
";

    #[test]
    fn an_autofs_mount_point_is_an_automount_whatever_is_stacked_on_it() {
        assert!(autofs_at(TABLE, Path::new("/net")));
        assert!(
            autofs_at(TABLE, Path::new("/proc/sys/fs/binfmt_misc")),
            "a direct mount point is one even after its real filesystem is mounted on top"
        );
        assert!(
            autofs_at(TABLE, Path::new("/mnt/my maps")),
            "an escaped space in the mount point is decoded"
        );
    }

    #[test]
    fn other_mounts_and_directories_inside_a_map_are_not_automount_points() {
        assert!(!autofs_at(TABLE, Path::new("/")));
        assert!(!autofs_at(TABLE, Path::new("/net/host")));
        assert!(!autofs_at(TABLE, Path::new("/ne")));
        assert!(!autofs_at(TABLE, Path::new("/tmp")));
    }

    #[test]
    fn an_ordinary_directory_is_not_an_automount() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!is_automount_directory(dir.path()));
    }

    /// Reads the real table, so a parser that never matches cannot pass by agreeing with
    /// fixtures written to suit it.
    #[test]
    fn this_hosts_own_autofs_mounts_are_found() {
        let table = std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
        let Some(point) = table.lines().find_map(|l| {
            let f: Vec<&str> = l.split(' ').collect();
            let dash = f.iter().position(|x| *x == "-")?;
            (f.get(dash + 1) == Some(&"autofs") && !f[4].contains('\\')).then(|| f[4].to_owned())
        }) else {
            println!("SKIPPED [TDD 2.7]: this host has no autofs mount");
            return;
        };
        assert!(is_automount_directory(Path::new(&point)), "{point}");
    }
}
