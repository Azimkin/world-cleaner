from __future__ import annotations

import argparse
import gzip
import json
import os
import shutil
import struct
import sys
import time
import tomllib
import zlib
from concurrent.futures import ProcessPoolExecutor, as_completed
from pathlib import Path


SECTOR_BYTES = 4096
HEADER_BYTES = SECTOR_BYTES * 2


def _u16(data: memoryview, pos: int) -> tuple[int, int]:
    return struct.unpack_from(">H", data, pos)[0], pos + 2


def _i32(data: memoryview, pos: int) -> tuple[int, int]:
    return struct.unpack_from(">i", data, pos)[0], pos + 4


def _string(data: memoryview, pos: int) -> tuple[str, int]:
    length, pos = _u16(data, pos)
    end = pos + length
    return bytes(data[pos:end]).decode("utf-8"), end


def _skip_payload(data: memoryview, pos: int, tag: int) -> int:
    if tag in (1,):
        return pos + 1
    if tag in (2,):
        return pos + 2
    if tag in (3, 5):
        return pos + 4
    if tag in (4, 6):
        return pos + 8
    if tag == 7:
        length, pos = _i32(data, pos)
        return pos + length
    if tag == 8:
        length, pos = _u16(data, pos)
        return pos + length
    if tag == 9:
        element_tag = data[pos]
        length, pos = _i32(data, pos + 1)
        for _ in range(length):
            pos = _skip_payload(data, pos, element_tag)
        return pos
    if tag == 10:
        while data[pos] != 0:
            child_tag = data[pos]
            _, pos = _string(data, pos + 1)
            pos = _skip_payload(data, pos, child_tag)
        return pos + 1
    if tag == 11:
        length, pos = _i32(data, pos)
        return pos + length * 4
    if tag == 12:
        length, pos = _i32(data, pos)
        return pos + length * 8
    raise ValueError(f"unknown NBT tag {tag}")


def _palette_entry_names(data: memoryview, pos: int) -> tuple[list[str], int]:
    names: list[str] = []
    while data[pos] != 0:
        tag = data[pos]
        key, pos = _string(data, pos + 1)
        if tag == 8 and key == "Name":
            name, pos = _string(data, pos)
            names.append(name)
        else:
            pos = _skip_payload(data, pos, tag)
    return names, pos + 1


def _palette_list(data: memoryview, pos: int) -> tuple[list[str], int]:
    element_tag = data[pos]
    length, pos = _i32(data, pos + 1)
    names: list[str] = []
    for _ in range(length):
        if element_tag == 10:
            entry_names, pos = _palette_entry_names(data, pos)
            names.extend(entry_names)
        else:
            pos = _skip_payload(data, pos, element_tag)
    return names, pos


def _block_states(data: memoryview, pos: int) -> tuple[list[str], int]:
    names: list[str] = []
    while data[pos] != 0:
        tag = data[pos]
        key, pos = _string(data, pos + 1)
        if tag == 9 and key == "palette":
            names, pos = _palette_list(data, pos)
        else:
            pos = _skip_payload(data, pos, tag)
    return names, pos + 1


def _section_matches(data: memoryview, pos: int, whitelist: frozenset[str]) -> tuple[bool, int]:
    matches = True
    while data[pos] != 0:
        tag = data[pos]
        key, pos = _string(data, pos + 1)
        if tag == 10 and key == "block_states":
            names, pos = _block_states(data, pos)
            if any(name not in whitelist for name in names):
                matches = False
        else:
            pos = _skip_payload(data, pos, tag)
    return matches, pos + 1


def _chunk_matches(data: bytes, whitelist: frozenset[str]) -> bool:
    nbt = memoryview(data)
    if not nbt:
        return True
    if nbt[0] != 10:
        raise ValueError("chunk NBT root is not a compound")
    _, pos = _string(nbt, 1)
    matches = True
    while nbt[pos] != 0:
        tag = nbt[pos]
        key, pos = _string(nbt, pos + 1)
        if tag == 9 and key == "sections":
            element_tag = nbt[pos]
            length, pos = _i32(nbt, pos + 1)
            for _ in range(length):
                if element_tag == 10:
                    section_matches, pos = _section_matches(nbt, pos, whitelist)
                    matches = matches and section_matches
                else:
                    pos = _skip_payload(nbt, pos, element_tag)
        else:
            pos = _skip_payload(nbt, pos, tag)
    return matches


def _decompress(payload: bytes, compression: int) -> bytes:
    if compression == 1:
        return gzip.decompress(payload)
    if compression == 2:
        return zlib.decompress(payload)
    if compression == 3:
        return payload
    raise ValueError(f"unsupported compression type {compression}")


def _external_chunk_path(region: Path, index: int) -> Path:
    _, region_x, region_z = region.stem.split(".")
    local_x, local_z = index % 32, index // 32
    return region.with_name(f"c.{int(region_x) * 32 + local_x}.{int(region_z) * 32 + local_z}.mcc")


def inspect_region(path_string: str, whitelist: frozenset[str]) -> tuple[bool, int, int, str | None]:
    path = Path(path_string)
    try:
        if path.stat().st_size == 0:
            return True, 0, 0, None
        with path.open("rb") as region:
            header = region.read(HEADER_BYTES)
            if len(header) < HEADER_BYTES:
                raise ValueError("region file is smaller than its 8192-byte header")
            removable = True
            chunks_checked = 0
            chunks_matching = 0
            for index in range(1024):
                location = struct.unpack_from(">I", header, index * 4)[0]
                sector_offset, sector_count = location >> 8, location & 0xFF
                if sector_offset == 0 or sector_count == 0:
                    continue
                region.seek(sector_offset * SECTOR_BYTES)
                length_bytes = region.read(4)
                if len(length_bytes) != 4:
                    raise ValueError(f"chunk {index}: missing length")
                length = struct.unpack(">I", length_bytes)[0]
                compression_byte = region.read(1)
                if not compression_byte:
                    raise ValueError(f"chunk {index}: missing compression type")
                compression = compression_byte[0]
                if compression & 0x80:
                    payload = _external_chunk_path(path, index).read_bytes()
                else:
                    if length < 1 or length > sector_count * SECTOR_BYTES - 4:
                        raise ValueError(f"chunk {index}: invalid length")
                    payload = region.read(length - 1)
                    if len(payload) != length - 1:
                        raise ValueError(f"chunk {index}: truncated payload")
                chunks_checked += 1
                if _chunk_matches(_decompress(payload, compression & 0x7F), whitelist):
                    chunks_matching += 1
                else:
                    removable = False
            return removable, chunks_checked, chunks_matching, None
    except Exception as error:  # A malformed region is retained, never moved.
        return False, 0, 0, str(error)


def find_region_dir(world_path: Path) -> Path:
    standard = world_path / "region"
    modern = world_path / "dimensions" / "minecraft" / "overworld" / "region"
    if standard.is_dir():
        return standard
    if modern.is_dir():
        return modern
    raise ValueError(f"no region directory found in {world_path}")


def move_region(source: Path, output_dir: Path) -> None:
    output_dir.mkdir(parents=True, exist_ok=True)
    destination = output_dir / source.name
    if destination.exists():
        raise FileExistsError(f"destination already exists: {destination}")
    shutil.move(str(source), str(destination))


def main() -> int:
    parser = argparse.ArgumentParser(description="Move Minecraft regions containing only whitelisted blocks.")
    parser.add_argument("config", nargs="?", default="config.toml", help="path to TOML config")
    args = parser.parse_args()
    config_path = Path(args.config).resolve()
    with config_path.open("rb") as config_file:
        config = tomllib.load(config_file)
    world_path = Path(config["world_path"])
    output_dir = Path(config["out_path"])
    if not world_path.is_absolute():
        world_path = config_path.parent / world_path
    if not output_dir.is_absolute():
        output_dir = config_path.parent / output_dir
    whitelist = frozenset(config["block_whitelist"])
    region_dir = find_region_dir(world_path)
    cache_path = config_path.with_name("world-cleaner-cache.json")
    cache = json.loads(cache_path.read_text(encoding="utf-8")) if cache_path.exists() else {}
    not_empty = set(cache.get("not_empty", []))
    paths = sorted(region_dir.glob("*.mca"))
    queued = [path for path in paths if path.name not in not_empty]
    print(f"Found {len(paths)} regions: {len(queued)} queued, {len(paths) - len(queued)} skipped from cache.")

    started = time.perf_counter()
    checked = kept = moved = chunks_checked = chunks_matching = 0
    errors: list[str] = []
    kept_names: list[str] = []
    deleted_names: list[str] = []
    progress_step = max(len(queued) // 100, 1)
    with ProcessPoolExecutor(max_workers=os.cpu_count() or 1) as pool:
        futures = {pool.submit(inspect_region, str(path), whitelist): path for path in queued}
        for completed, future in enumerate(as_completed(futures), 1):
            path = futures[future]
            removable, chunk_count, matching_count, error = future.result()
            checked += 1
            chunks_checked += chunk_count
            chunks_matching += matching_count
            if error:
                errors.append(f"{path.name}: {error}")
            elif removable:
                try:
                    move_region(path, output_dir)
                    moved += 1
                    deleted_names.append(path.name)
                except OSError as move_error:
                    errors.append(f"{path.name}: {move_error}")
            else:
                kept += 1
                kept_names.append(path.name)
            if completed % progress_step == 0 or completed == len(queued):
                print(f"Progress: {completed}/{len(queued)}")

    not_empty.update(kept_names)
    cache_path.write_text(json.dumps({"not_empty": sorted(not_empty), "last_deleted": deleted_names}, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    elapsed = time.perf_counter() - started
    average = elapsed / checked if checked else 0.0
    print(f"\nDone.\nRegions: checked {checked}, matched filter {moved}, kept {kept}, skipped from cache {len(paths) - len(queued)}.\nChunks: checked {chunks_checked}, matched filter {chunks_matching}.\nTime: {elapsed:.2f}s total, {average:.4f}s per checked region.\nErrors: {len(errors)}")
    for error in errors:
        print(f"ERROR: {error}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
