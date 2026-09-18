use crate::config::{read_cache, read_config, verify_config, write_cache};
use crate::region_reader::process_regions;
use std::env;
use std::time::Instant;

mod config;
mod region_reader;

fn main() {
    let config_path = env::args().nth(1).unwrap_or(String::from("./config.toml"));

    let config = read_config(&config_path);
    verify_config(&config);
    let cache_path = std::path::Path::new(&config_path).with_file_name("world-cleaner-cache.toml");
    let mut cache = read_cache(&cache_path);
    cache.last_deleted.clear();

    let started = Instant::now();
    let result =
        process_regions(&config, &cache.not_empty).expect("Could not scan region directory");
    cache.not_empty.extend(result.kept_names);
    cache.last_deleted = result.deleted_names;
    write_cache(&cache_path, &cache);
    let elapsed = started.elapsed();
    let average = if result.checked == 0 {
        0.0
    } else {
        elapsed.as_secs_f64() / result.checked as f64
    };

    println!(
        "\nDone.\nRegions: checked {}, matched filter {}, kept {}, skipped from cache {}.\nChunks: checked {}, matched filter {}.\nTime: {:.2}s total, {:.4}s per checked region.\nErrors: {}",
        result.checked,
        result.moved,
        result.kept,
        result.skipped,
        result.chunks_checked,
        result.chunks_matching_filter,
        elapsed.as_secs_f64(),
        average,
        result.errors.len()
    );
    for error in result.errors {
        eprintln!("ERROR: {error}");
    }
}
