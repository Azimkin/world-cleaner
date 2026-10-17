use crate::config::{Cache, Config, threads_in_use, write_cache};
use fastanvil::Region;
use rayon::prelude::*;
use serde::Deserialize;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Deserialize)]
struct Chunk {
    #[serde(default)]
    sections: Vec<Section>,
}
#[derive(Deserialize)]
struct Section {
    #[serde(default)]
    block_states: BlockStates,
}
#[derive(Default, Deserialize)]
struct BlockStates {
    #[serde(default)]
    palette: Vec<PaletteEntry>,
}
#[derive(Deserialize)]
struct PaletteEntry {
    #[serde(rename = "Name")]
    name: String,
}

pub struct ScanResult {
    pub checked: usize,
    pub kept: usize,
    pub moved: usize,
    pub skipped: usize,
    pub errors: Vec<String>,
    pub chunks_checked: usize,
    pub chunks_matching_filter: usize,
}

struct RegionDecision {
    removable: bool,
    chunks_checked: usize,
    chunks_matching_filter: usize,
}

fn region_dir(world_path: &str) -> Result<PathBuf, String> {
    let world = Path::new(world_path);
    let standard = world.join("region");
    let modern = world
        .join("dimensions")
        .join("minecraft")
        .join("overworld")
        .join("region");
    if standard.is_dir() {
        Ok(standard)
    } else if modern.is_dir() {
        Ok(modern)
    } else {
        Err(format!("No region directory found in {}", world.display()))
    }
}

fn region_files(config: &Config) -> Result<Vec<PathBuf>, String> {
    Ok(fs::read_dir(region_dir(&config.world_path)?)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "mca"))
        .collect())
}

// Every palette in the region is read. A region matches only if their union is
// a subset of the whitelist; no individual chunk is ever moved on its own.
fn inspect_region(path: &Path, whitelist: &HashSet<String>) -> Result<RegionDecision, String> {
    if fs::metadata(path).map_err(|e| e.to_string())?.len() == 0 {
        return Ok(RegionDecision {
            removable: true,
            chunks_checked: 0,
            chunks_matching_filter: 0,
        });
    }

    let mut region = Region::from_stream(fs::File::open(path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let mut decision = RegionDecision {
        removable: true,
        chunks_checked: 0,
        chunks_matching_filter: 0,
    };
    for raw_chunk in region.iter() {
        let raw_chunk = raw_chunk.map_err(|e| e.to_string())?;
        let chunk: Chunk = fastnbt::from_bytes(&raw_chunk.data).map_err(|e| e.to_string())?;
        decision.chunks_checked += 1;
        let mut chunk_matches = true;
        for section in chunk.sections {
            for block in section.block_states.palette {
                if !whitelist.contains(&block.name) {
                    chunk_matches = false;
                }
            }
        }
        if chunk_matches {
            decision.chunks_matching_filter += 1;
        } else {
            decision.removable = false;
        }
    }
    Ok(decision)
}

fn move_file(source: &Path, output_dir: &Path) -> Result<(), String> {
    fs::create_dir_all(output_dir).map_err(|e| e.to_string())?;
    let destination = output_dir.join(source.file_name().ok_or("Region path has no filename")?);
    if destination.exists() {
        return Err(format!(
            "Destination already exists: {}",
            destination.display()
        ));
    }
    match fs::rename(source, &destination) {
        Ok(()) => Ok(()),
        Err(rename_error) => {
            fs::copy(source, &destination)
                .map_err(|e| format!("{rename_error}; fallback copy failed: {e}"))?;
            fs::remove_file(source).map_err(|e| {
                format!(
                    "Copied to {}, but could not remove source: {e}",
                    destination.display()
                )
            })
        }
    }
}

pub fn process_regions(
    config: &Config,
    cache: &mut Cache,
    cache_path: &Path,
) -> Result<ScanResult, String> {
    let whitelist: HashSet<String> = config.block_whitelist.iter().cloned().collect();
    let (skipped, paths): (Vec<_>, Vec<_>) = region_files(config)?.into_iter().partition(|path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| cache.not_empty.contains(name))
    });
    println!(
        "Found {} regions: {} queued, {} skipped from cache.",
        paths.len() + skipped.len(),
        paths.len(),
        skipped.len()
    );
    let mut result = ScanResult {
        checked: 0,
        kept: 0,
        moved: 0,
        skipped: skipped.len(),
        errors: vec![],
        chunks_checked: 0,
        chunks_matching_filter: 0,
    };
    if paths.is_empty() {
        write_cache(cache_path, cache);
        return Ok(result);
    }

    let total = paths.len();
    let progress_step = (total / 100).max(1);
    let completed = AtomicUsize::new(0);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads_in_use(config))
        .build()
        .map_err(|error| error.to_string())?;
    for chunk in paths.chunks(config.flush_every) {
        let decisions: Vec<_> = pool.install(|| {
            chunk
                .par_iter()
                .map(|path| {
                    let decision = inspect_region(path, &whitelist);
                    let current = completed.fetch_add(1, Ordering::Relaxed) + 1;
                    if current % progress_step == 0 || current == total {
                        println!("Progress: {current}/{total}");
                    }
                    (path, decision)
                })
                .collect()
        });
        for (path, decision) in decisions {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("<invalid name>")
                .to_owned();
            match decision {
                Ok(decision) if !decision.removable => {
                    result.chunks_checked += decision.chunks_checked;
                    result.chunks_matching_filter += decision.chunks_matching_filter;
                    result.kept += 1;
                    cache.not_empty.insert(name);
                }
                Ok(decision) => match move_file(path, Path::new(&config.out_path)) {
                    Ok(()) => {
                        result.chunks_checked += decision.chunks_checked;
                        result.chunks_matching_filter += decision.chunks_matching_filter;
                        result.moved += 1;
                        cache.last_deleted.push(name);
                    }
                    Err(error) => {
                        result.chunks_checked += decision.chunks_checked;
                        result.chunks_matching_filter += decision.chunks_matching_filter;
                        result.errors.push(format!("{name}: {error}"));
                    }
                },
                Err(error) => result.errors.push(format!("{name}: {error}")),
            }
        }
        result.checked += chunk.len();
        write_cache(cache_path, cache);
        let done = completed.load(Ordering::Relaxed);
        println!(
            "Saved cache: {done}/{total} scanned, {} moved.",
            result.moved
        );
    }
    Ok(result)
}
