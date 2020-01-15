use std::fs;
use std::io;
use std::path::Path;
use std::env;

pub fn run(config: Config) -> io::Result<()> {
    // let contents = fs::read_to_string(config.filename)?;
    let dir = Path::new(&config.dir);
    visit_dirs(dir, &print_file_info)

    // for line in results {
    //     println!("{}", line);
    // }

    // Ok(())
}

fn print_file_info(file: &fs::DirEntry) {
    println!("File: {:?} size: {}", file.path(), file.metadata().expect("No metadata").len());
}

fn visit_dirs(dir: &Path, callback: &dyn Fn(&fs::DirEntry)) -> io::Result<()> {
    if dir.is_dir() {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            let metadata = entry.metadata()?;
            if path.is_dir() && metadata.is_dir() {
                visit_dirs(&path, callback)?;
            } else {
                callback(&entry);
            }
        }
    }
    Ok(())
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
    use std::fs::File;

    #[test]
    fn visit_dirs_test_dir() {
        let dir = Path::new("./target/test_dir");
        std::fs::remove_dir_all(dir).unwrap_or_else(|error| {
            if error.kind() != io::ErrorKind::NotFound{
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

        // visit_dirs(dir, &|file| println!("File: {:?} size: {}", file.path(), file.metadata().expect("No metadata").len())).unwrap();

        let mut out = File::open(dir.join("dup1a.txt")).unwrap();
        let mut buf = [0; 128 * 1024];
        let mut hasher = blake3::Hasher::new();
        loop {
            let length = out.read(&mut buf).unwrap();
            if length == 0 {
                break;
            }
            hasher.update(& buf);
        }
        let hash1 = hasher.finalize();

        // let hash1 = blake3::hash(b"foobarbaz");
        println!("hex: {}", hash1.to_hex());
        assert_eq!("fcc85134f1e140988a686dbd857f9dcf453cfbfc986f0fcfbb987a0436a1cd42", hash1.to_hex().as_str());
    }
}