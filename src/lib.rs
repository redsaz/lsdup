use crate::lsdup::config::Config;
use crate::lsdup::filevisitor::AllInFileVisitor;
use crate::lsdup::filevisitor::FileVisitor;
use std::fs;
use std::io;
use std::path::Path;
use std::string::String;

pub mod lsdup;

pub fn run(config: &Config) -> io::Result<AllInFileVisitor> {
    let dirs = &config.dirs;
    let mut dups = AllInFileVisitor::new(&config);

    for dir in dirs {
        if let Err(foo) = visit_dirs(dir, &mut dups, &config) {
            return Err(foo);
        }
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
                            if let Err(e) = visit_dirs(&path, visitor, config) {
                                eprintln!("Skipping directory {:?}.\nReason: {}", path, e);
                            }
                        } else if metadata.is_file() {
                            visitor.visit(path);
                        } else {
                            eprintln!(
                                "Skipping {:?}. It is not a directory or regular file.",
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

    #[cfg(target_family = "unix")]
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

    #[cfg(target_family = "unix")]
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
    #[cfg(target_family = "unix")]
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
