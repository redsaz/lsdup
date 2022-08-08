// Device+Inode number are used to identify hard linked data.
#[derive(
    std::hash::Hash,
    std::cmp::Eq,
    std::cmp::PartialEq,
    std::cmp::Ord,
    std::cmp::PartialOrd,
    std::fmt::Debug,
)]
pub struct DevIno {
    dev: u64,
    ino: u64,
}

impl DevIno {
    #[cfg(target_family = "unix")]
    pub fn from(meta: &dyn std::os::unix::fs::MetadataExt) -> DevIno {
        let dev = meta.dev();
        let ino = meta.ino();
        DevIno { dev, ino }
    }

    #[cfg(target_family = "windows")]
    pub fn from(meta: &fs::Metadata) -> DevIno {
        // Don't worry, I know this is horrible.
        let dev = 0;
        let ino = 0;
        DevIno { dev, ino }
    }
}
