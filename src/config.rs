use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::Path;

#[derive(Deserialize, Serialize)]
pub struct Config {
    pub(crate) world_path: String,
    pub(crate) out_path: String,
    /// `0` uses every available thread.
    #[serde(default)]
    pub(crate) threads: usize,
    /// Regions to scan before moving matches and saving the cache.
    #[serde(default = "default_flush_every")]
    pub(crate) flush_every: usize,
    pub(crate) block_whitelist: Vec<String>,
}

fn default_flush_every() -> usize {
    512
}

#[derive(Default, Deserialize, Serialize)]
pub struct Cache {
    #[serde(default)]
    pub not_empty: HashSet<String>,
    #[serde(default)]
    pub last_deleted: Vec<String>,
}

fn example_config() -> Config {
    Config {
        world_path: "./world".into(),
        out_path: "./out".into(),
        threads: 0,
        flush_every: default_flush_every(),
        block_whitelist: vec![
            "minecraft:grass_block".into(),
            "minecraft:dirt".into(),
            "minecraft:bedrock".into(),
            "minecraft:air".into(),
        ],
    }
}

pub fn available_threads() -> usize {
    std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1)
}

/// `threads = 0` means the scan uses every available thread.
pub fn threads_in_use(config: &Config) -> usize {
    if config.threads == 0 {
        available_threads()
    } else {
        config.threads
    }
}

pub fn print_threads(config: &Config) {
    println!(
        "Threads: {} available, {} in use.",
        available_threads(),
        threads_in_use(config)
    );
}

pub fn read_config(path: &str) -> Config {
    if !Path::new(path).is_file() {
        if path == "./config.toml" {
            let example = example_config();
            fs::write(path, toml::to_string_pretty(&example).unwrap())
                .expect("Could not create example config");
            println!("Created {path}.");
            print_threads(&example);
        }
        panic!("Unable to read config file: {path}");
    }
    toml::from_str(&fs::read_to_string(path).expect("Could not read config"))
        .expect("Invalid config TOML")
}

pub fn verify_config(config: &Config) {
    if config.flush_every == 0 {
        panic!("flush_every must be greater than 0");
    }
    if !Path::new(&config.world_path).is_dir() {
        panic!("World directory does not exist or is not a directory");
    }
    if Path::new(&config.out_path).exists() && !Path::new(&config.out_path).is_dir() {
        panic!("Output path exists but is not a directory");
    }
}

pub fn read_cache(path: &Path) -> Cache {
    match fs::read_to_string(path) {
        Ok(contents) => toml::from_str(&contents).expect("Invalid cache TOML"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Cache::default(),
        Err(error) => panic!("Could not read cache: {error}"),
    }
}

pub fn write_cache(path: &Path, cache: &Cache) {
    fs::write(path, toml::to_string_pretty(cache).unwrap()).expect("Could not write cache");
}
