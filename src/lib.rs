use std::collections::HashMap;
use std::env;
use std::fs;
use std::fs::File;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::vec::Vec;
use arrayvec::ArrayString;
use memmap::MmapOptions;

#[derive(std::hash::Hash,std::cmp::Eq,std::cmp::PartialEq)]
struct LenHash {
    len: u64,
    hash: [u8; 32]
}

impl LenHash {
    pub fn from(len: u64, hash: [u8; 32]) -> LenHash {
        LenHash {
            len,
            hash
        }
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

trait FileVisitor {
    fn visit(&mut self, file: PathBuf);
}

struct BySizeFileVisitor {
    size_files_map: HashMap<u64, Vec<Box<Path>>>,
}

impl BySizeFileVisitor {
    fn new() -> BySizeFileVisitor {
        BySizeFileVisitor {
            size_files_map: HashMap::new(),
        }
    }
}

impl FileVisitor for BySizeFileVisitor {
    fn visit(&mut self, file: PathBuf) {
        if let Err(e) = file.metadata() {
            eprintln!("Error: Could not get metadata for {:?}: {}", file, e);
            return;
        }
        match file.metadata() {
            Ok(meta) => {
                let size = meta.len();
                eprintln!("File: {:?} size: {}", file, size);
                let paths = self.size_files_map.entry(size).or_insert_with(Vec::new);
                paths.push(file.into_boxed_path());
            }
            Err(e) => {
                eprintln!("Error: Could not get metadata for {:?}: {}", file, e);
            }
        }
    }
}

// impl IntoIterator for BySizeFileVisitor {
//     type Item = Box<PathBuf>;
//     type IntoIter = std::iter::Flatten<Self::Item>;

//     fn into_iter(self) -> Self::IntoIter {
//         self.size_files_map.values().filter(|v| v.len() > 1).flatten()
//     }
// }
impl<'a> IntoIterator for &'a BySizeFileVisitor {
    type Item = &'a Vec<Box<Path>>;
    type IntoIter = std::collections::hash_map::Values<'a, u64, Vec<Box<Path>>>;

    fn into_iter(self) -> Self::IntoIter {
        self.size_files_map.values()
    }
}

struct ByHashFileVisitor {
    hash_files_map: HashMap<LenHash, Vec<Box<Path>>>,
}

impl ByHashFileVisitor {
    fn new() -> ByHashFileVisitor {
        ByHashFileVisitor {
            hash_files_map: HashMap::new(),
        }
    }
}

impl FileVisitor for ByHashFileVisitor {
    fn visit(&mut self, file: PathBuf) {
        // Group all visited files by hash.
        eprintln!("File: {:?} size: {}", file, file.metadata().unwrap().len());
        let hash = hash_contents(&file);
        eprintln!("\thash: {}", hash.to_hex());
        let paths = self.hash_files_map.entry(hash).or_insert_with(Vec::new);
        paths.push(file.into_boxed_path());
    }
}

impl<'a> IntoIterator for &'a ByHashFileVisitor {
    type Item = (
        &'a LenHash,
        &'a std::vec::Vec<std::boxed::Box<std::path::Path>>,
    );
    type IntoIter = std::collections::hash_map::Iter<
        'a,
        LenHash,
        std::vec::Vec<std::boxed::Box<std::path::Path>>,
    >;

    fn into_iter(self) -> Self::IntoIter {
        self.hash_files_map.iter()
    }
}

pub fn run(config: Config) -> io::Result<()> {
    // let contents = fs::read_to_string(config.filename)?;
    let dir = Path::new(&config.dir);
    let mut dups = BySizeFileVisitor::new();
    eprintln!("Gathering files by filesize for {:?}...", dir);
    visit_dirs(dir, &mut dups);

    // Go through all vecs, skipping the ones with only one entry. Hash the rest.
    let mut byhash = ByHashFileVisitor::new();
    eprintln!("\nGathering files by hash for {:?}...", dir);
    for x in &dups {
        if x.len() < 2 {
            continue;
        }

        eprintln!("Group of {} files.", x.len());

        for y in x {
            byhash.visit(y.to_path_buf());
        }
    }

    // THEN GO THROUGH *THOSE* ENTRIES. SKIP ONES WITH ONLY ONE ITEM. WHAT REMAINS ARE DUPES.
    // ORDER THEM AS APPROPRIATE AND OUTPUT THE RESULTS.

    for x in &byhash {
        if x.1.len() < 2 {
            continue;
        }

        println!("\nSize: {}  Hash: {}", x.0.len(), x.0.to_hex());
        for y in x.1 {
            println!("{}", y.to_string_lossy());
        }
    }

    Ok(())
}

fn hash_contents(file: &PathBuf) -> LenHash {
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
    let mmap = unsafe { MmapOptions::new().map(&file).expect("Could not memmap file.") };

    let mut hasher = blake3::Hasher::new();
    hasher.update(&mmap);

    LenHash::from(size, hasher.finalize().into())
}

fn print_file_info(file: &fs::DirEntry) {
    let size = match file.metadata() {
        Ok(n) => n.len().to_string(),
        Err(..) => "No metadata".to_string(),
    };
    eprintln!("File: {:?} size: {}", file.path(), size);
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

    #[test]
    fn visit_dirs_test_dir() {
        let dir = Path::new("./target/test_dir");
        std::fs::remove_dir_all(dir).unwrap_or_else(|error| {
            if error.kind() != io::ErrorKind::NotFound {
                panic!("Problem removing old directory: {:?}", error);
            }
        });

        std::fs::create_dir(dir).unwrap_or_else(|error| {
            if error.kind() != io::ErrorKind::AlreadyExists {
                panic!("Problem creating directory: {:?}", error);
            }
        });

        let contents1 = b"Contents1";
        let mut dup1a = File::create(dir.join("dup1a.txt")).unwrap();
        dup1a.write_all(contents1).unwrap();
        let mut dup1b = File::create(dir.join("dup1b.txt")).unwrap();
        dup1b.write_all(contents1).unwrap();

        // visit_dirs(dir, &|file| eprintln!("File: {:?} size: {}", file.path(), file.metadata().expect("No metadata").len())).unwrap();

        let mut out = File::open(dir.join("dup1a.txt")).unwrap();
        let mut buf = [0; 128 * 1024];
        let mut hasher = blake3::Hasher::new();
        loop {
            let length = out.read(&mut buf).unwrap();
            if length == 0 {
                break;
            }
            hasher.update(&buf);
        }
        let hash1 = hasher.finalize();

        // let hash1 = blake3::hash(b"foobarbaz");
        eprintln!("hex: {}", hash1.to_hex());
        assert_eq!(
            "fcc85134f1e140988a686dbd857f9dcf453cfbfc986f0fcfbb987a0436a1cd42",
            hash1.to_hex().as_str()
        );
    }
}
