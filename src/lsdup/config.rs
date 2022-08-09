use std::path::PathBuf;
use clap::{App, Arg};

#[derive(std::fmt::Debug)]
pub struct Config {
    pub dirs: Vec<PathBuf>,
    pub verbosity: u8,
}

impl Config {
    pub fn new() -> Result<Config, &'static str> {
        let matches = App::new("List Duplicates")
            .version("0.1.0")
            .author("redsaz <redsaz@gmail.com>")
            .about("Finds files with duplicate contents")
            .arg(
                Arg::with_name("DIR")
                    .help("The directory to scan")
                    .multiple(true)
                    .last(true)
                    .default_value("."),
            )
            .arg(
                Arg::with_name("verbose")
                    .short('v')
                    .long("verbose")
                    .multiple(true)
                    .help("Sets the level of verbosity, repeat for more verbosity"),
            )
            .get_matches();

        let val_strings = matches
            .get_many::<String>("DIR")
            .map(|vals| vals.collect::<Vec<_>>())
            .unwrap_or_default();
        let dirs = val_strings
            .into_iter()
            .map(|val| PathBuf::from(val))
            .collect();

        let verbosity = matches.occurrences_of("verbose") as u8;

        Ok(Config { dirs, verbosity })
    }
}
