use arrayvec::ArrayString;
use memmap::MmapOptions;
use std::collections::BTreeMap;
use std::fs;
use std::fs::File;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::string::String;
use std::vec::Vec;
use clap::{Arg, App};

#[derive(std::hash::Hash, std::cmp::Eq, std::cmp::PartialEq, std::fmt::Debug)]
pub struct LenHash {
    len: u64,
    hash: [u8; 32],
}

impl LenHash {
    pub fn from(len: u64, hash: [u8; 32]) -> LenHash {
        LenHash { len, hash }
    }

    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn to_hex(&self) -> ArrayString<[u8; 32 * 2]> {
        // As done in Blake3 to_hex function.
        let mut s = ArrayString::new();
        let table = b"0123456789abcdef";
        for &b in self.hash.iter() {
            s.push(table[(b >> 4) as usize] as char);
            s.push(table[(b & 0xf) as usize] as char);
        }
        s
    }
}

impl Ord for LenHash {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Compare other with self, instead of self with other,
        // so the ordering becomes largest-to-smallest
        other
            .len
            .cmp(&self.len)
            .then_with(|| other.hash.cmp(&self.hash))
    }
}

impl PartialOrd for LenHash {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

// Device+Inode number are used to identify hard linked data.
#[derive(
    std::hash::Hash,
    std::cmp::Eq,
    std::cmp::PartialEq,
    std::cmp::Ord,
    std::cmp::PartialOrd,
    std::fmt::Debug,
)]
struct DevIno {
    dev: u64,
    ino: u64,
}

impl DevIno {
    pub fn from(meta: &dyn std::os::unix::fs::MetadataExt) -> DevIno {
        let dev = meta.dev();
        let ino = meta.ino();
        DevIno { dev, ino }
    }
}

// len, hash, and first file.
#[derive(std::fmt::Debug)]
struct LinkedFile {
    len: u64,
    hash: Option<[u8; 32]>,
    first: Option<PathBuf>,
}

impl LinkedFile {
    pub fn init(len: u64, first: PathBuf) -> LinkedFile {
        let hash = None;
        let first = Some(first);
        LinkedFile { len, hash, first }
    }
}

trait FileVisitor {
    fn visit(&mut self, file: PathBuf);
}

#[derive(std::fmt::Debug)]
pub struct AllInFileVisitor<'a> {
    config: &'a Config,

    // The first file for the size is stored here. If another file with
    // the same size comes along, then the first file will get hashed,
    // And the Some is replaced with None. Then the second file is hashed.
    // Any later files will get hashed.
    size_firstfile_map: BTreeMap<u64, Option<PathBuf>>,

    // If there are two or more files of a given size found, then they
    // will be hashed and placed in this map.
    hash_files_map: BTreeMap<LenHash, Vec<PathBuf>>,

    // Files that are hardlinked are treated specially, because the user
    // usually (unless an option is set otherwise) doesn't want to consider
    // hardlinks as duplicate. Also we don't want to hash two or more times
    // if we know its all pointing to the same data.
    hardlinks_map: BTreeMap<DevIno, LinkedFile>,

    // Total bytes of all the files processed.
    total_file_bytes: u64,

    // Total number of files processed.
    num_files: u32,
}

impl<'a> AllInFileVisitor<'a> {
    fn new(config: &'a Config) -> AllInFileVisitor {
        AllInFileVisitor {
            config,
            size_firstfile_map: BTreeMap::new(),
            hash_files_map: BTreeMap::new(),
            hardlinks_map: BTreeMap::new(),
            total_file_bytes: 0,
            num_files: 0,
        }
    }

    pub fn num_files(&self) -> u32 {
        self.num_files
    }

    pub fn total_file_bytes(&self) -> u64 {
        self.total_file_bytes
    }
}

impl<'a> FileVisitor for AllInFileVisitor<'a> {
    fn visit(&mut self, file: PathBuf) {
        if let Err(e) = file.metadata() {
            eprintln!("Error: Could not get metadata for {:?}: {}", file, e);
            return;
        }
        match file.metadata() {
            Ok(meta) => {
                let size = meta.len();

                if self.config.verbosity > 0 {
                    eprintln!("File: {:?} size: {}", file, size);
                }

                // If the inode that the file points at has at least one other file
                // pointing at it, we should treat it special so that we don't hash
                // the same data twice.
                if has_hardlinks(&meta) {
                    let inode = DevIno::from(&meta);
                    let e = self.hardlinks_map.get_mut(&inode);
                    // If there is already an entry for the dev+inode, then toss or
                    // calculate hash, according to CLI option
                    if let Some(_files) = e {
                        // TODO Right now there is no CLI option to list hard links as
                        // dupes. So we toss it.
                        // I think something like this?
                        // if list_hardlinks_as_dupes {
                        //     if let Some(first) = hlink.first {
                        //         let hash = hash_contents_path(&first);
                        //         let paths = self.hash_files_map.entry(hash).or_insert_with(Vec::new);
                        //         paths.push(first)
                        //         hlink.first = None;
                        //         hlink.hash = hash;
                        //     } else {
                        //         let paths = self.hash_files_map.entry(hlink.hash)
                        //         paths.push(file);
                        //     }
                        // }
                        return;
                    } else {
                        let files = LinkedFile::init(size, file.to_owned());
                        self.hardlinks_map.insert(inode, files);
                    }
                }

                self.total_file_bytes += size;
                self.num_files += 1;

                let e = self.size_firstfile_map.get(&size);
                // If there is already an entry for the given size...
                if let Some(inner_opt) = e {
                    // ...and there is already a file with the given byte size, then hash that file
                    // first, before hashing the current file.
                    if let Some(original) = inner_opt {
                        let hash = hash_contents_path(&original);
                        if self.config.verbosity > 0 {
                            eprintln!("\thash: {}", hash.to_hex());
                        }
                        let paths = self.hash_files_map.entry(hash).or_insert_with(Vec::new);
                        paths.push(original.clone());
                        // (and replace the Some with None, so it won't be hashed again)
                        self.size_firstfile_map.insert(size, None);
                    }
                    // ...now hash the current file.
                    let hash = hash_contents_path(&file);
                    if self.config.verbosity > 0{
                        eprintln!("\thash: {}", hash.to_hex());
                    }
                    let paths = self.hash_files_map.entry(hash).or_insert_with(Vec::new);
                    paths.push(file);
                } else {
                    // Since there isn't an entry for the given size, that means this is the first
                    // file with that size. Put it in the size map so that if another file with the
                    // same size is encountered, it can be hashed too.
                    self.size_firstfile_map.insert(size, Some(file));
                }
            }
            Err(e) => {
                eprintln!("Error: Could not get metadata for {:?}: {}", file, e);
            }
        }
    }
}

impl<'a> IntoIterator for &'a AllInFileVisitor<'a> {
    type Item = (&'a LenHash, &'a std::vec::Vec<PathBuf>);
    type IntoIter = std::iter::Filter<
        std::collections::btree_map::Iter<'a, LenHash, std::vec::Vec<PathBuf>>,
        for<'r> fn(&'r (&LenHash, &std::vec::Vec<std::path::PathBuf>)) -> bool,
    >;

    fn into_iter(self) -> Self::IntoIter {
        self.hash_files_map.iter().filter(only_with_dupes)
    }
}

fn only_with_dupes<'r>(x: &'r (&LenHash, &std::vec::Vec<PathBuf>)) -> bool {
    x.1.len() > 1
}

pub fn run(config: &Config) -> io::Result<AllInFileVisitor> {
    let dir = Path::new(&config.dir);
    let mut dups = AllInFileVisitor::new(&config);

    if let Err(foo) = visit_dirs(dir, &mut dups, &config) {
        return Err(foo);
    }

    Ok(dups)
}

pub fn print_results(dups: &AllInFileVisitor) {
    let mut num_dups = 0;
    let mut dup_bytes: u64 = 0;
    for x in dups {
        println!(
            "\nSize: {}  Hash: {}",
            friendly_bytes(x.0.len()),
            x.0.to_hex()
        );
        for y in x.1 {
            println!("{}", y.to_string_lossy());
        }
        num_dups += x.1.len() - 1;
        dup_bytes += (x.1.len() - 1) as u64 * (x.0.len() as u64);
    }
    eprintln!(
        "{} files, {} analyzed.",
        &dups.num_files(),
        friendly_bytes(dups.total_file_bytes())
    );
    eprintln!(
        "{} duplicate files, {} of duplicates.",
        &num_dups,
        friendly_bytes(dup_bytes)
    );

    eprintln!("{} sets of duplicates.", dups.into_iter().count());
}

fn friendly_bytes(bytes: u64) -> String {
    if bytes >= 1 << 30 {
        let value = (bytes as f64) / (1024 * 1024 * 1024) as f64;
        return format!("{:.1} GB", value);
    } else if bytes >= 1 << 20 {
        let value = (bytes as f64) / (1024 * 1024) as f64;
        return format!("{:.1} MB", value);
    } else if bytes >= 1 << 10 {
        let value = (bytes as f64) / 1024.0;
        return format!("{:.1} kB", value);
    }
    format!("{} B", bytes)
}

// If this file is a hardlink, then return true.
fn has_hardlinks(meta: &dyn std::os::unix::fs::MetadataExt) -> bool {
    meta.nlink() > 1
}

fn hash_contents_path(file: &Path) -> LenHash {
    let file = File::open(file).expect("Could not open file for reading.");
    let size = file.metadata().expect("Could not get file size.").len();

    if size >= 16384 && size <= isize::max_value() as u64 {
        hash_contents_mmap(size, &file)
    } else {
        hash_contents_file(size, file)
    }
}

fn hash_contents_file(size: u64, file: File) -> LenHash {
    let mut file = file;
    let mut hasher = blake3::Hasher::new();
    std::io::copy(&mut file, &mut hasher).expect("Could not hash file contents.");

    LenHash::from(size, hasher.finalize().into())
}

fn hash_contents_mmap(size: u64, file: &File) -> LenHash {
    let mmap = unsafe {
        MmapOptions::new()
            .map(&file)
            .expect("Could not memmap file.")
    };

    let mut hasher = blake3::Hasher::new();
    hasher.update(&mmap);

    LenHash::from(size, hasher.finalize().into())
}

fn visit_dirs(dir: &Path, visitor: &mut dyn FileVisitor, config: &Config) -> io::Result<()> {
    let dir_iter = fs::read_dir(dir)?;
    for entry in dir_iter {
        match entry {
            Ok(entry) => {
                let path = entry.path();
                match entry.metadata() {
                    Ok(metadata) => {
                        // Only visit real (non-symlinked) directories
                        if path.is_dir() && metadata.is_dir() {
                            visit_dirs(&path, visitor, config)?;
                        } else if metadata.is_file() {
                            visitor.visit(path);
                        } else {
                            eprintln!(
                                "Skipping this, which is neither a directory nor a file: {:?}",
                                path
                            );
                        }
                    }
                    Err(e) => eprintln!("Skipping {:?}.\nReason: {}", entry, e),
                }
            }
            Err(e) => eprintln!("Skipping entry in directory {:?}.\nReason: {}", dir, e),
        }
    }
    Ok(())
}

#[derive(std::fmt::Debug)]
pub struct Config {
    pub dir: String,
    pub verbosity: u8,
}

impl Config {
    pub fn new() -> Result<Config, &'static str> {
        let matches = App::new("List Duplicates")
            .version("0.1.0")
            .author("redsaz <redsaz@gmail.com>")
            .about("Finds files with duplicate contents")
            .arg(Arg::with_name("DIR").help("The directory to scan").index(1))
            .arg(
                Arg::with_name("verbose")
                    .short("v")
                    .long("verbose")
                    .multiple(true)
                    .help("Sets the level of verbosity, repeat for more verbosity"),
            )
            .get_matches();

        let dir = String::from(matches.value_of("DIR").unwrap_or("."));
        println!("Value for dir: {}", dir);

        let verbosity = matches.occurrences_of("verbose") as u8;

        Ok(Config { dir, verbosity })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::prelude::*;

    fn create_dir_all(target_dir: &Path) {
        std::fs::create_dir_all(target_dir).unwrap_or_else(|error| {
            if error.kind() != io::ErrorKind::AlreadyExists {
                panic!("Problem creating directory: {:?}", error);
            }
        });
    }

    #[test]
    fn test_run() {
        // Given a directory with two files,
        let target_dir = Path::new("./target/test_dir/two_files");
        create_dir_all(target_dir);

        // and both files are identical,
        let orig_path = target_dir.join(Path::new("a.txt"));
        {
            let mut original = File::create(&orig_path).unwrap();
            original
                .write_all(b"Contents for a test of two files of identical content. This should create a \"duplicate group\" with one file being marked as original, and the other as a duplicate.")
                .expect("Could not write data for file.");
        }
        let dupe_path = target_dir.join(Path::new("b.txt"));
        let _ = std::fs::copy(orig_path.as_path(), dupe_path.as_path())
            .expect("Could not write data for file.");

        // and the configuration is to analyze that directory,
        let config = Config {
            dir: target_dir.to_string_lossy().to_string(),
            verbosity: 0,
        };

        // When dupes are analyzed for that directory,
        let dupes = run(&config).expect("Could not analyze directory.");

        // Then one group with two files should be listed,
        // and "a.txt" is the original and "b.txt" is the duplicate because
        // alphabetically a.txt comes first.
        assert_eq!(1, dupes.into_iter().count());
        let mut iter = dupes.into_iter();
        let group = iter.next().unwrap();
        assert_eq!(2, group.1.len());
        assert_eq!(orig_path.as_path(), group.1[0].as_path());
        assert_eq!(dupe_path.as_path(), group.1[1].as_path());
        assert!(iter.next().is_none(), "Only one dupe group should exist.");
    }

    #[test]
    fn test_run_two_original_files_different_length() {
        // Given a directory with two files,
        let target_dir = Path::new("./target/test_dir/two_files_different_length");
        create_dir_all(target_dir);

        // and both files are different content and length,
        let orig1_path = target_dir.join(Path::new("a.txt"));
        {
            let mut original = File::create(&orig1_path).unwrap();
            original
                .write_all(b"Contents for a test of two files of different content. Both have different sizes as well. 1sdoerknsad")
                .expect("Could not write data for file.");
        }
        let orig2_path = target_dir.join(Path::new("b.txt"));
        {
            let mut original = File::create(&orig2_path).unwrap();
            original
                .write_all(b"Contents for a test of two files of different content. Both have different sizes as well. 2sdoer")
                .expect("Could not write data for file.");
        }

        // and the configuration is to analyze that directory,
        let config = Config {
            dir: target_dir.to_string_lossy().to_string(),
            verbosity: 0,
        };

        // When dupes are analyzed for that directory,
        let dupes = run(&config).expect("Could not analyze directory.");

        // Then zero groups should be listed,
        // because both files are different.
        assert_eq!(0, dupes.into_iter().count());
    }

    #[test]
    fn test_run_two_original_files_same_length() {
        // Given a directory with two files,
        let target_dir = Path::new("./target/test_dir/two_files_same_length");
        create_dir_all(target_dir);

        // and both files are different content but same length,
        let orig1_path = target_dir.join(Path::new("a.txt"));
        {
            let mut original = File::create(&orig1_path).unwrap();
            original
                .write_all(b"Contents for a test of two files of different content. Both have different sizes as well. 1zcn,eiudn")
                .expect("Could not write data for file.");
        }
        let orig2_path = target_dir.join(Path::new("b.txt"));
        {
            let mut original = File::create(&orig2_path).unwrap();
            original
                .write_all(b"Contents for a test of two files of different content. Both have different sizes as well. 2zcn,eiudn")
                .expect("Could not write data for file.");
        }

        // and the configuration is to analyze that directory,
        let config = Config {
            dir: target_dir.to_string_lossy().to_string(),
            verbosity: 0,
        };

        // When dupes are analyzed for that directory,
        let dupes = run(&config).expect("Could not analyze directory.");

        // Then zero groups should be listed,
        // because both files are different.
        assert_eq!(0, dupes.into_iter().count());
    }

    #[test]
    fn test_run_one_file() {
        // Given a directory with one file,
        let target_dir = Path::new("./target/test_dir/one_file");
        create_dir_all(target_dir);

        let orig_path = target_dir.join(Path::new("a.txt"));
        {
            let mut original = File::create(&orig_path).unwrap();
            original
                .write_all(b"Contents for a test of one file.")
                .expect("Could not write data for file.");
        }

        // and the configuration is to analyze that directory,
        let config = Config {
            dir: target_dir.to_string_lossy().to_string(),
            verbosity: 0,
        };

        // When dupes are analyzed for that directory,
        let dupes = run(&config).expect("Could not analyze directory.");

        // Then no files should be listed, since a directory with one file cannot have a dupe.
        assert_eq!(0, dupes.into_iter().count());
    }

    #[test]
    fn test_run_zero_files() {
        // Given a directory with zero files,
        let target_dir = Path::new("./target/test_dir/zero_files");
        create_dir_all(target_dir);

        // and the configuration is to analyze that directory,
        let config = Config {
            dir: target_dir.to_string_lossy().to_string(),
            verbosity: 0,
        };

        // When dupes are analyzed for that directory,
        let dupes = run(&config).expect("Could not analyze directory.");

        // Then no files should be listed, since a directory with zero files cannot have a dupe.
        assert_eq!(0, dupes.into_iter().count());
    }

    #[test]
    fn test_run_bogus_dir() {
        // Given a directory that doesn't exist,
        let target_dir = Path::new("./target/test_dir/bogus_dir");

        // and the configuration is to analyze that directory,
        let config = Config {
            dir: target_dir.to_string_lossy().to_string(),
            verbosity: 0,
        };

        // When dupes are analyzed for that directory,
        // Then an error is returned indicating the directory is not found.
        let err = run(&config)
            .expect_err("Should error when attempting to analyze a non-existing directory.");
        assert_eq!(io::ErrorKind::NotFound, err.kind());
    }

    #[test]
    fn test_run_non_dir_path() {
        // Given a file instead of a directory,
        let target_dir = Path::new("./target/test_dir/non_dir_path");
        create_dir_all(target_dir);

        let non_dir_path = target_dir.join(Path::new("non_dir"));
        {
            let mut non_dir = File::create(&non_dir_path).unwrap();
            non_dir
                .write_all(b"This is a file, not a directory.")
                .expect("Could not write data for file.");
        }

        // and the configuration is to analyze that file,
        let config = Config {
            dir: non_dir_path.to_string_lossy().to_string(),
            verbosity: 0,
        };

        // When dupes are analyzed for that non-directory,
        // Then an error is returned indicating the path given is not to a directory.
        let err = run(&config)
            .expect_err("Should error when attempting to analyze a path that isn't a directory.");
        assert_eq!(io::ErrorKind::Other, err.kind());
    }

    #[test]
    fn test_run_hard_link() {
        // Given a directory with two files,
        // and one file has original data,
        let target_dir = Path::new("./target/test_dir/hard_links");
        create_dir_all(target_dir);

        let orig_path = target_dir.join(Path::new("a.txt"));
        {
            let mut original = File::create(&orig_path).unwrap();
            original
                .write_all(b"Contents for non-duplicated data. kjhkjh")
                .expect("Could not write data for file.");
        }

        // and another file is hardlinked to that data,
        let hlink_path = target_dir.join(Path::new("a-hardlink.txt"));
        std::fs::hard_link(&orig_path, &hlink_path).unwrap_or_else(|error| {
            if error.kind() != io::ErrorKind::AlreadyExists {
                panic!("Problem creating hardlink: {:?}", error);
            }
        });

        // and the configuration is to analyze that directory, not listing hardlinks as duplicates,
        let config = Config {
            dir: target_dir.to_string_lossy().to_string(),
            verbosity: 0,
        };

        // When dupes are analyzed for that directory,
        let dupes = run(&config).expect("Could not analyze directory.");

        // Then no files should be listed, since only the original file and a hard link were found.
        assert_eq!(0, dupes.into_iter().count());
    }

    #[test]
    fn test_run_hard_links_dupes() {
        // Given a directory with four files files,
        // and one file has original data,
        let target_dir = Path::new("./target/test_dir/hard_links_dupes");
        create_dir_all(target_dir);

        let orig_path = target_dir.join(Path::new("a.txt"));
        let data = b"Contents for non-duplicated data. zcvzxcv";
        {
            let mut original = File::create(&orig_path).unwrap();
            original
                .write_all(data)
                .expect("Could not write data for file.");
        }

        // and another file is hardlinked to that data,
        let hlink_path = target_dir.join(Path::new("a-hardlink.txt"));
        std::fs::hard_link(&orig_path, &hlink_path).unwrap_or_else(|error| {
            if error.kind() != io::ErrorKind::AlreadyExists {
                panic!("Problem creating hardlink: {:?}", error);
            }
        });

        // and a third file has the same contents as the first file, but isn't hard linked to it,
        let dupe_path = target_dir.join(Path::new("b.txt"));
        {
            let mut duplicate = File::create(&dupe_path).unwrap();
            duplicate
                .write_all(data)
                .expect("Could not write data for file.");
        }

        // and a fourth file is hardlinked to that data,
        let dupe_hlink_path = target_dir.join(Path::new("b-hardlink.txt"));
        std::fs::hard_link(&dupe_path, &dupe_hlink_path).unwrap_or_else(|error| {
            if error.kind() != io::ErrorKind::AlreadyExists {
                panic!("Problem creating hardlink: {:?}", error);
            }
        });

        // and the configuration is to analyze that directory, not listing hardlinks as duplicates,
        let config = Config {
            dir: target_dir.to_string_lossy().to_string(),
            verbosity: 0,
        };

        // When dupes are analyzed for that directory,
        let dupes = run(&config).expect("Could not analyze directory.");

        // Then there should be a duplicates group listing the first file as the original and the
        // third file as a duplicate
        assert_eq!(1, dupes.into_iter().count());
        let mut iter = dupes.into_iter();
        let group = iter.next().unwrap();
        assert_eq!(2, group.1.len());
        assert_eq!(orig_path.as_path(), group.1[0].as_path());
        assert_eq!(dupe_path.as_path(), group.1[1].as_path());
        assert!(iter.next().is_none(), "Only one dupe group should exist.");
    }

    #[test]
    fn test_run_symlink() {
        // Given a directory with two files,
        // and one file has original data,
        let target_dir = Path::new("./target/test_dir/sym_links");
        create_dir_all(target_dir);

        let orig_file = Path::new("a.txt");
        let orig_path = target_dir.join(&orig_file);
        {
            let mut original = File::create(&orig_path).unwrap();
            original
                .write_all(b"Contents for non-duplicated data. qwelkrj")
                .expect("Could not write data for file.");
        }

        // and another file is symlinked to that data,
        let hlink_path = target_dir.join(Path::new("a-symlink.txt"));
        std::os::unix::fs::symlink(&orig_file, &hlink_path).unwrap_or_else(|error| {
            if error.kind() != io::ErrorKind::AlreadyExists {
                panic!("Problem creating symlink: {:?}", error);
            }
        });

        // and the configuration is to analyze that directory, not inspecting symlinked files or directories,
        let config = Config {
            dir: target_dir.to_string_lossy().to_string(),
            verbosity: 0,
        };

        // When dupes are analyzed for that directory,
        let dupes = run(&config).expect("Could not analyze directory.");

        // Then no files should be listed, since only the original file and a symlink were found.
        assert_eq!(0, dupes.into_iter().count());
    }
}
