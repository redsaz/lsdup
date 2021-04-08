const HELP: &str = "\
List Duplicates
USAGE:
  lsdup [OPTIONS] file1 [file2...fileN]
FLAGS:
  -h, --help            Prints help information
OPTIONS:
  --dummy text          Does nothing
ARGS:
  file1                 File to hash
";

#[derive(Debug)]
struct AppArgs {
    dummy: Option<String>,
    file: std::path::PathBuf,
}

fn main() {
    let args = match parse_args() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Error: {}.", e);
            std::process::exit(1);
        }
    };

    println!("{:#?}", args);

    let mut hasher = blake3::Hasher::new();
    hasher.update(b"foo");
    hasher.update(b"bar");
    hasher.update(b"baz");
    let hash2 = hasher.finalize();

    println!("{}", hash2.to_hex());
}

fn parse_args() -> Result<AppArgs, pico_args::Error> {
    let mut pargs = pico_args::Arguments::from_env();

    if pargs.contains(["-h", "--help"]) {
        print!("{}", HELP);
        std::process::exit(0);
    }

    let args = AppArgs {
        dummy: pargs.opt_value_from_str("--dummy")?,
        file: pargs.free_from_str()?,
    };

    Ok(args)
}
