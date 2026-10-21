//! What a disk's format lets an installation hold: Mac builds keep symbolic
//! links in their bundles, which ExFAT and FAT do not hold, FAT holds no file
//! of 4 GiB or more, and macOS only reads NTFS.
use std::path::Path;

/// A disk's format, as macOS names its file system.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Format {
    Apfs,
    Hfs,
    ExFat,
    Fat,
    Ntfs,
    /// Any other, such as a network share's: taken to hold everything.
    Other(String),
}

impl Format {
    /// The format of the disk `path`, or its nearest existing parent, lies
    /// on.
    #[cfg(unix)]
    pub fn of(path: &Path) -> Option<Self> {
        let existing = path.ancestors().find(|ancestor| ancestor.exists())?;
        let stats = rustix::fs::statfs(existing).ok()?;
        let name: Vec<u8> = stats
            .f_fstypename
            .iter()
            .take_while(|byte| **byte != 0)
            .map(|byte| *byte as u8)
            .collect();
        Some(Self::named(&String::from_utf8_lossy(&name)))
    }

    /// The format macOS names `name`, such as `apfs` or `msdos`.
    pub fn named(name: &str) -> Self {
        match name {
            "apfs" => Self::Apfs,
            "hfs" => Self::Hfs,
            "exfat" => Self::ExFat,
            "msdos" => Self::Fat,
            "ntfs" => Self::Ntfs,
            other => Self::Other(other.to_owned()),
        }
    }

    /// Whether it holds symbolic links, which Mac builds' bundles have.
    pub fn holds_links(&self) -> bool {
        !matches!(self, Self::ExFat | Self::Fat)
    }

    /// The largest file it holds, when it is smaller than any game's.
    pub fn file_limit(&self) -> Option<u64> {
        matches!(self, Self::Fat).then_some(u32::MAX as u64)
    }

    /// Whether macOS writes to it.
    pub fn writable(&self) -> bool {
        !matches!(self, Self::Ntfs)
    }

    /// Whether some game cannot be installed there.
    pub fn limited(&self) -> bool {
        !self.holds_links() || self.file_limit().is_some() || !self.writable()
    }

    /// Its name, as Disk Utility shows it.
    pub fn title(&self) -> &str {
        match self {
            Self::Apfs => "APFS",
            Self::Hfs => "Mac OS Extended",
            Self::ExFat => "ExFAT",
            Self::Fat => "MS-DOS (FAT)",
            Self::Ntfs => "NTFS",
            Self::Other(name) => name,
        }
    }

    /// Whether a disk write that failed with `message`, a system error as
    /// Rust shows it, ran into what this format cannot hold: a link it
    /// refuses, a file too large, or a disk only read.
    pub fn refused(&self, message: &str) -> bool {
        const REFUSALS: [&str; 4] = [
            "(os error 1)",
            "(os error 27)",
            "(os error 30)",
            "(os error 45)",
        ];
        self.limited() && REFUSALS.iter().any(|refusal| message.contains(refusal))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_formats_hold_everything_and_others_have_their_limits() {
        for name in ["apfs", "hfs", "smbfs"] {
            let format = Format::named(name);
            assert!(!format.limited(), "{name}");
            assert!(!format.refused("Operation not supported (os error 45)"));
        }
        let exfat = Format::named("exfat");
        assert!(!exfat.holds_links() && exfat.file_limit().is_none() && exfat.writable());
        let fat = Format::named("msdos");
        assert_eq!(fat.file_limit(), Some(4_294_967_295));
        assert!(!Format::named("ntfs").writable());
        assert!(exfat.refused("backend: Operation not permitted (os error 1)"));
        assert!(fat.refused("File too large (os error 27)"));
        assert!(!exfat.refused("No such file or directory (os error 2)"));
        assert!(!exfat.refused("Permission denied (os error 13)"));
    }

    #[test]
    fn a_missing_folder_reads_its_nearest_parent_s_format() {
        let root = tempfile::tempdir().unwrap();
        let format = Format::of(&root.path().join("not/yet/made")).unwrap();
        assert_eq!(format, Format::of(root.path()).unwrap());
    }
}
