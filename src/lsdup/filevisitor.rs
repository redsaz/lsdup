use crate::lsdup::config::Config;
use crate::lsdup::devino::DevIno;
use crate::lsdup::lenhash::LenHash;
use std::path::PathBuf;
use indicatif::ProgressBar;
use std::collections::BTreeMap;
use console::Term;
use std::io;
use std::path::Path;
use std::fs::File;
use memmap::MmapOptions;

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

pub trait FileVisitor {
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

    // Displays progress/stats if attached to a terminal
    progress_bar: ProgressBar,

    // Allows printing if actually a terminal
    term: Term,
}

impl<'a> AllInFileVisitor<'a> {
    pub fn new(config: &'a Config) -> AllInFileVisitor {
        AllInFileVisitor {
            config,
            size_firstfile_map: BTreeMap::new(),
            hash_files_map: BTreeMap::new(),
            hardlinks_map: BTreeMap::new(),
            total_file_bytes: 0,
            num_files: 0,
            progress_bar: ProgressBar::new_spinner(),
            term: console::Term::stderr(),
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
        if self.term.features().is_attended() {
            let width = self.term.size_checked().unwrap_or((25, 40)).1 as usize;
            let msg = file.to_str().unwrap_or("<invalid utf8>");
            if width > 4 && msg.len() >= width - 3 {
                for i in (0..(width - 3)).rev() {
                    if msg.is_char_boundary(i) {
                        self.progress_bar.set_message(msg.get(..i).unwrap_or(""));
                        break;
                    }
                }
            } else {
                self.progress_bar.set_message(msg);
            }
        }
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
                        match hash_contents_path(&original) {
                            Ok(hash) => {
                                if self.config.verbosity > 0 {
                                    eprintln!("\thash: {}", hash.to_hex());
                                }
                                let paths =
                                    self.hash_files_map.entry(hash).or_insert_with(Vec::new);
                                paths.push(original.clone());
                                // (and replace the Some with None, so it won't be hashed again)
                                self.size_firstfile_map.insert(size, None);
                            }
                            Err(e) => {
                                eprintln!("Error: Could not hash {:?}: {}", original, e);
                            }
                        }
                    }
                    // ...now hash the current file.
                    match hash_contents_path(&file) {
                        Ok(hash) => {
                            if self.config.verbosity > 0 {
                                eprintln!("\thash: {}", hash.to_hex());
                            }
                            let paths = self.hash_files_map.entry(hash).or_insert_with(Vec::new);
                            paths.push(file);
                        }
                        Err(e) => {
                            eprintln!("Error: Could not hash {:?}: {}", file, e);
                        }
                    }
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

// If this file is a hardlink, then return true.
#[cfg(target_family = "unix")]
fn has_hardlinks(meta: &dyn std::os::unix::fs::MetadataExt) -> bool {
    meta.nlink() > 1
}

// If this file is a hardlink, then return true.
#[cfg(target_family = "windows")]
fn has_hardlinks(meta: &fs::Metadata) -> bool {
    // This is possible in Windows, but for now skip it.
    false
}

fn hash_contents_path(file: &Path) -> io::Result<LenHash> {
    let file = File::open(file)?;
    let size = file.metadata()?.len();

    if size >= 16384 && size <= isize::max_value() as u64 {
        hash_contents_mmap(size, &file)
    } else {
        hash_contents_file(size, file)
    }
}

fn hash_contents_file(size: u64, file: File) -> io::Result<LenHash> {
    let mut file = file;
    let mut hasher = blake3::Hasher::new();
    std::io::copy(&mut file, &mut hasher)?;

    Ok(LenHash::from(size, hasher.finalize().into()))
}

fn hash_contents_mmap(size: u64, file: &File) -> io::Result<LenHash> {
    let mmap = unsafe { MmapOptions::new().map(&file)? };

    let mut hasher = blake3::Hasher::new();
    hasher.update(&mmap);

    Ok(LenHash::from(size, hasher.finalize().into()))
}

fn only_with_dupes<'r>(x: &'r (&LenHash, &std::vec::Vec<PathBuf>)) -> bool {
    x.1.len() > 1
}
