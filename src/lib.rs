use arrayvec::ArrayString;
use memmap::MmapOptions;
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::fs::File;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::vec::Vec;

#[derive(std::hash::Hash, std::cmp::Eq, std::cmp::PartialEq)]
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
    std::hash::Hash, std::cmp::Eq, std::cmp::PartialEq, std::cmp::Ord, std::cmp::PartialOrd,
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

pub struct AllInFileVisitor {
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
}

impl AllInFileVisitor {
    fn new() -> AllInFileVisitor {
        AllInFileVisitor {
            size_firstfile_map: BTreeMap::new(),
            hash_files_map: BTreeMap::new(),
            hardlinks_map: BTreeMap::new(),
        }
    }
}

impl FileVisitor for AllInFileVisitor {
    fn visit(&mut self, file: PathBuf) {
        if let Err(e) = file.metadata() {
            eprintln!("Error: Could not get metadata for {:?}: {}", file, e);
            return;
        }
        match file.metadata() {
            Ok(meta) => {
                let size = meta.len();

                eprintln!("File: {:?} size: {}", file, size);

                // If the inode that the file points at has at least one other file
                // pointing at it, we should treat it special so that we don't hash
                // the same data twice.
                if has_hardlinks(&meta) {
                    let inode = DevIno::from(&meta);
                    let e = self.hardlinks_map.get_mut(&inode);
                    // If there is already an entry for the dev+inode, then toss or
                    // calculate hash, according to CLI option
                    if let Some(files) = e {
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
                        let len = meta.len();
                        let files = LinkedFile::init(len, file.to_owned());
                        self.hardlinks_map.insert(inode, files);
                    }
                }

                let e = self.size_firstfile_map.get(&size);
                // If there is already an entry for the given size...
                if let Some(inner_opt) = e {
                    // ...and there is already a file with the given byte size, then hash that file
                    // first, before hashing the current file.
                    if let Some(original) = inner_opt {
                        let hash = hash_contents_path(&original);
                        eprintln!("\thash: {}", hash.to_hex());
                        let paths = self.hash_files_map.entry(hash).or_insert_with(Vec::new);
                        paths.push(original.clone());
                        // (and replace the Some with None, so it won't be hashed again)
                        self.size_firstfile_map.insert(size, None);
                    }
                    // ...now hash the current file.
                    let hash = hash_contents_path(&file);
                    eprintln!("\thash: {}", hash.to_hex());
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

impl<'a> IntoIterator for &'a AllInFileVisitor {
    type Item = (&'a LenHash, &'a std::vec::Vec<PathBuf>);
    type IntoIter = std::collections::btree_map::Iter<'a, LenHash, std::vec::Vec<PathBuf>>;

    fn into_iter(self) -> Self::IntoIter {
        self.hash_files_map.iter()
    }
}

pub fn run(config: &Config) -> io::Result<AllInFileVisitor> {
    let dir = Path::new(&config.dir);
    let mut dups = AllInFileVisitor::new();
    eprintln!("Analyzing for {:?}...", dir);
    visit_dirs(dir, &mut dups);

    // Iterate through all of the hashed files map, return only the ones that have two or more.
    for x in &dups {
        if x.1.len() < 2 {
            continue;
        }

        println!("\nSize: {}  Hash: {}", x.0.len(), x.0.to_hex());
        for y in x.1 {
            println!("{}", y.to_string_lossy());
        }
    }

    Ok(dups)
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

fn visit_dirs(dir: &Path, visitor: &mut dyn FileVisitor) {
    if dir.is_dir() {
        match fs::read_dir(dir) {
            Ok(dir_iter) => {
                for entry in dir_iter {
                    match entry {
                        Ok(entry) => {
                            let path = entry.path();
                            match entry.metadata() {
                                Ok(metadata) => {
                                    if path.is_dir() && metadata.is_dir() {
                                        visit_dirs(&path, visitor);
                                    } else if metadata.is_file() {
                                        visitor.visit(path);
                                    } else {
                                        eprintln!("I'm not sure what this is: {:?}", path);
                                    }
                                }
                                Err(e) => eprintln!("Skipping {:?}.\nReason: {}", entry, e),
                            }
                        }
                        Err(e) => {
                            eprintln!("Skipping entry in directory {:?}.\nReason: {}", dir, e)
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("Skipping directory {:?}.\nReason: {}", dir, e);
            }
        }
    }
}

pub struct Config {
    pub dir: String,
}

impl Config {
    pub fn new(args: &mut env::Args) -> Result<Config, &'static str> {
        args.next(); // Skip the executable name

        let dir = match args.next() {
            Some(arg) => arg,
            None => return Err("didn't get a directory"),
        };

        Ok(Config { dir })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::prelude::*;
    use std::os::unix::fs::MetadataExt;

    #[test]
    fn visit_dirs_test_dir() {
        // let dir = Path::new("./target/test_dir");
        // std::fs::remove_dir_all(dir).unwrap_or_else(|error| {
        //     if error.kind() != io::ErrorKind::NotFound {
        //         panic!("Problem removing old directory: {:?}", error);
        //     }
        // });

        // std::fs::create_dir(dir).unwrap_or_else(|error| {
        //     if error.kind() != io::ErrorKind::AlreadyExists {
        //         panic!("Problem creating directory: {:?}", error);
        //     }
        // });

        // let contents1 = b"Contents1";
        // let mut dup1a = File::create(dir.join("dup1a.txt")).unwrap();
        // dup1a.write_all(contents1).unwrap();
        // let mut dup1b = File::create(dir.join("dup1b.txt")).unwrap();
        // dup1b.write_all(contents1).unwrap();

        // let mut out = File::open(dir.join("dup1a.txt")).unwrap();
        // let mut buf = [0; 128 * 1024];
        // let mut hasher = blake3::Hasher::new();
        // loop {
        //     let length = out.read(&mut buf).unwrap();
        //     if length == 0 {
        //         break;
        //     }
        //     hasher.update(&buf);
        // }
        // let hash1 = hasher.finalize();

        // // let hash1 = blake3::hash(b"foobarbaz");
        // eprintln!("hex: {}", hash1.to_hex());
        // assert_eq!(
        //     "fcc85134f1e140988a686dbd857f9dcf453cfbfc986f0fcfbb987a0436a1cd42",
        //     hash1.to_hex().as_str()
        // );
    }

    #[test]
    fn hard_link() {
        // Given a directory with two files,
        // and one file has original data,
        let target_dir = Path::new("./target/test_dir/hard_links");
        std::fs::create_dir(target_dir).unwrap_or_else(|error| {
            if error.kind() != io::ErrorKind::AlreadyExists {
                panic!("Problem creating directory: {:?}", error);
            }
        });

        let orig_path = target_dir.join(Path::new("a.txt"));
        {
        let mut original = File::create(&orig_path).unwrap();
        original.write_all(b"Contents for non-duplicated data. kjhkjh").expect("Could not write data for file.");
        }

        // and another file is hardlinked to that data,
        let hlink_path = target_dir.join(Path::new("a-hardlink.txt"));
        std::fs::hard_link(&orig_path, &hlink_path).unwrap_or_else(|error| {
            if error.kind() != io::ErrorKind::AlreadyExists {
                panic!("Problem creating hardlink: {:?}", error);
            }
        });

        // and the configuration is to analyze that directory, not listing hardlinks as duplicates,
        let config = Config{dir: target_dir.to_string_lossy().to_string() };

        // When dupes are analyzed for that directory,
        run(&config).expect("Could not analyze directory.");

        // Then no files should be listed, since only the original file and a hard link were found.

    }
}
