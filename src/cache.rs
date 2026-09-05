//! On-disk cache for seed-based visualizations.
//!
//! When a `--seed` is given the whole pipeline is deterministic: the seeded
//! shuffle produces the base arrangement and running the chosen sort over it
//! produces an identical event list every time. That data (the shuffled `base`
//! plus the recorded `events`) is serialized to a file keyed by
//! `(seed, image hash, grid size, algorithm, sort key)` so a later run with
//! the same seed + settings loads instantly instead of re-sorting.
//!
//! Format is a simple little-endian binary blob (no external dependencies):
//!
//! ```text
//! magic   "ISORTVC1"           8 bytes
//! version u32                   4 bytes
//! seed    u64                   8 bytes
//! image_hash u64                8 bytes
//! cols    u32                   4 bytes
//! rows    u32                   4 bytes
//! algo    u8                    1 byte
//! key     u8                    1 byte
//! n       u32                   4 bytes
//! base    n * u32               4*n bytes
//! events  u64 count           8 bytes
//!         count * (tag u8, a u32, b u32)
//! ```
//!
//! Every read is bounds-checked and validated against the expected header;
//! any mismatch (incl. a partially-written file) is treated as a cache miss.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::sort::{Algo, Event};

const MAGIC: &[u8; 8] = b"ISORTVC1";
const VERSION: u32 = 1;

/// Identity of one cacheable visualization.
#[derive(Clone, Debug)]
pub struct CacheHeader {
    pub seed: u64,
    pub image_hash: u64,
    pub cols: u32,
    pub rows: u32,
    /// Algorithm id (Algo::code()), stable across runs.
    pub algo: u8,
    pub key: u8,
    pub n: u32,
}

/// The cached animation data.
pub struct Visualization {
    /// Initial (seeded-shuffled) arrangement: `base[i]` is the cell at slot `i`.
    pub base: Vec<usize>,
    /// The recorded sort events that take `base` to the sorted arrangement.
    pub events: Vec<Event>,
}

/// Where the cache lives unless overridden with `--cache-dir`.
pub fn default_dir() -> PathBuf {
    if let Some(x) = std::env::var_os("XDG_CACHE_HOME") {
        Path::new(&x).join("sort_visualizer")
    } else if let Some(h) = std::env::var_os("HOME") {
        Path::new(&h).join(".cache").join("sort_visualizer")
    } else {
        PathBuf::from(".sort-cache")
    }
}

pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

fn key_name(code: u8) -> &'static str {
    if code == 0 { "index" } else { "luma" }
}

/// Algorithm display name for cache filenames (bubble, insertion, ...).
fn algo_name(code: u8) -> &'static str {
    Algo::from_code(code).map(|a| a.code_str()).unwrap_or("unknown")
}

/// Full path of the cache file for a header.
pub fn cache_path(dir: &Path, header: &CacheHeader) -> PathBuf {
    dir.join(format!("{:016x}", header.image_hash))
        .join(format!(
            "s{}_c{}x{}_a{}_k{}.bin",
            header.seed,
            header.cols,
            header.rows,
            algo_name(header.algo),
            key_name(header.key)
        ))
}

fn rd<'a>(data: &'a [u8], pos: &mut usize, len: usize) -> Option<&'a [u8]> {
    let end = pos.checked_add(len)?;
    let slice = data.get(*pos..end)?;
    *pos = end;
    Some(slice)
}

fn rd_u32(data: &[u8], pos: &mut usize) -> Option<u32> {
    Some(u32::from_le_bytes(rd(data, pos, 4)?.try_into().ok()?))
}

fn rd_u64(data: &[u8], pos: &mut usize) -> Option<u64> {
    Some(u64::from_le_bytes(rd(data, pos, 8)?.try_into().ok()?))
}

/// Load a cached visualization, validating it fully against `expect`.
///
/// Returns `None` on missing files, format mismatches, or any out-of-range
/// index — i.e. every problem degenerates to "compute and re-cache".
pub fn load(path: &Path, expect: &CacheHeader) -> Option<Visualization> {
    let data = fs::read(path).ok()?;
    let mut pos = 0usize;
    if rd(&data, &mut pos, 8)? != MAGIC {
        return None;
    }
    if rd_u32(&data, &mut pos)? != VERSION {
        return None;
    }
    let seed = rd_u64(&data, &mut pos)?;
    let image_hash = rd_u64(&data, &mut pos)?;
    let cols = rd_u32(&data, &mut pos)?;
    let rows = rd_u32(&data, &mut pos)?;
    let algo = *rd(&data, &mut pos, 1)?.first()?;
    let key = *rd(&data, &mut pos, 1)?.first()?;
    let n = rd_u32(&data, &mut pos)?;

    if seed != expect.seed
        || image_hash != expect.image_hash
        || cols != expect.cols
        || rows != expect.rows
        || algo != expect.algo
        || key != expect.key
        || n != expect.n
    {
        return None;
    }

    let n_us = n as usize;
    let mut base = Vec::with_capacity(n_us);
    for _ in 0..n_us {
        let v = rd_u32(&data, &mut pos)? as usize;
        if v >= n_us {
            return None; // corrupt: cell id out of range
        }
        base.push(v);
    }

    let remaining = data.len().saturating_sub(pos);
    let count = rd_u64(&data, &mut pos)?;
    let count_us = usize::try_from(count).ok()?;
    // Each event occupies at least 9 bytes (tag + two u32s); a count that big
    // cannot belong to this file.
    if count_us.saturating_mul(9) > remaining.saturating_sub(8) {
        return None;
    }

    let mut events = Vec::with_capacity(count_us.min(1_000_000));
    for _ in 0..count_us {
        let tag = *rd(&data, &mut pos, 1)?.first()?;
        let a = rd_u32(&data, &mut pos)? as usize;
        let b = rd_u32(&data, &mut pos)? as usize;
        if a >= n_us || b >= n_us {
            return None; // corrupt: event index out of range
        }
        events.push(match tag {
            0 => Event::Cmp { a, b },
            1 => Event::Swap { a, b },
            2 => Event::Set { idx: a, value: b },
            _ => return None,
        });
    }

    Some(Visualization { base, events })
}

/// Persist a visualization. Creates parent directories as needed.
pub fn save(path: &Path, header: &CacheHeader, base: &[usize], events: &[Event]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut buf = Vec::new();
    buf.extend_from_slice(MAGIC);
    buf.extend_from_slice(&VERSION.to_le_bytes());
    buf.extend_from_slice(&header.seed.to_le_bytes());
    buf.extend_from_slice(&header.image_hash.to_le_bytes());
    buf.extend_from_slice(&header.cols.to_le_bytes());
    buf.extend_from_slice(&header.rows.to_le_bytes());
    buf.push(header.algo);
    buf.push(header.key);
    buf.extend_from_slice(&header.n.to_le_bytes());

    for &v in base {
        buf.extend_from_slice(&(v as u32).to_le_bytes());
    }

    buf.extend_from_slice(&(events.len() as u64).to_le_bytes());
    for e in events {
        let (tag, a, b) = match e {
            Event::Cmp { a, b } => (0u8, *a, *b),
            Event::Swap { a, b } => (1u8, *a, *b),
            Event::Set { idx, value } => (2u8, *idx, *value),
        };
        buf.push(tag);
        buf.extend_from_slice(&(a as u32).to_le_bytes());
        buf.extend_from_slice(&(b as u32).to_le_bytes());
    }

    fs::write(path, buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_header() -> CacheHeader {
        CacheHeader {
            seed: 42,
            image_hash: 0xdead_beef_cafe_f00d,
            cols: 4,
            rows: 3,
            algo: Algo::Bubble.code(),
            key: 0,
            n: 12,
        }
    }

    fn sample_visualization() -> Visualization {
        Visualization {
            base: vec![5, 2, 9, 0, 11, 3, 7, 1, 10, 4, 8, 6],
            events: vec![
                Event::Cmp { a: 0, b: 1 },
                Event::Swap { a: 0, b: 1 },
                Event::Set { idx: 3, value: 7 },
            ],
        }
    }

    fn temp_file(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "sort_cookie_test_{tag}_{}_{:x}.bin",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn roundtrip() {
        let header = sample_header();
        let viz = sample_visualization();
        let path = temp_file("roundtrip");

        save(&path, &header, &viz.base, &viz.events).expect("save");
        let loaded = load(&path, &header).expect("load");
        assert_eq!(loaded.base, viz.base);
        assert_eq!(loaded.events.len(), viz.events.len());
        for (a, b) in loaded.events.iter().zip(&viz.events) {
            match (a, b) {
                (Event::Cmp { a: x, b: y }, Event::Cmp { a, b })
                | (Event::Swap { a: x, b: y }, Event::Swap { a, b })
                | (Event::Set { idx: x, value: y }, Event::Set { idx: a, value: b }) => {
                    assert!((x, y) == (a, b), "mismatched event payload");
                }
                _ => panic!("event order/variant changed"),
            }
        }

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn header_mismatch_is_a_miss() {
        let header = sample_header();
        let viz = sample_visualization();
        let path = temp_file("header_mismatch");
        save(&path, &header, &viz.base, &viz.events).expect("save");

        let mut wrong = sample_header();
        wrong.seed = 99;
        assert!(load(&path, &wrong).is_none());

        wrong = sample_header();
        wrong.n = 13;
        assert!(load(&path, &wrong).is_none());

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn corrupted_payload_is_a_miss() {
        let header = sample_header();
        let viz = sample_visualization();
        let path = temp_file("corrupt");
        save(&path, &header, &viz.base, &viz.events).expect("save");

        let mut data = fs::read(&path).unwrap();
        // Corrupt something in the middle of the file.
        let mid = data.len() / 2;
        data[mid] ^= 0xFF;
        fs::write(&path, &data).unwrap();
        assert!(load(&path, &header).is_none());

        // A truncated file must also be a miss.
        let path2 = temp_file("trunc");
        save(&path2, &header, &viz.base, &viz.events).expect("save");
        let data2 = fs::read(&path2).unwrap();
        fs::write(&path2, &data2[..data2.len() / 2]).unwrap();
        assert!(load(&path2, &header).is_none());

        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&path2);
    }

    #[test]
    fn fnv_is_stable() {
        assert_eq!(fnv1a(b"hello world"), fnv1a(b"hello world"));
        assert_ne!(fnv1a(b"hello world"), fnv1a(b"hello worlD"));
    }
}