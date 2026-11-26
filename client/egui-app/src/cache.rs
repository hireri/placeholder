//! On-disk cache of audio chunks. The daemon only hands out stream URLs and the engine fetches the bytes itself,
//! so the cache sits under the engine's range requests: `<data>/cache/audio/<song-format>/<chunk>`.
//! Eviction drops the least recently read chunks once the size limit is passed.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

static LIMIT: AtomicU64 = AtomicU64::new(0);
static USAGE: Mutex<Option<(Instant, u64)>> = Mutex::new(None);

pub fn set_limit_mb(mb: u16) {
    LIMIT.store((mb as u64) << 20, Relaxed);
}

fn root() -> PathBuf {
    crate::backend::data_dir().join("cache").join("audio")
}

/// Stable name of one format of one song, from the stream URL's `id`, `itag` and `clen` (the URL itself expires).
fn dir(url: &str, chunk: u64) -> Option<PathBuf> {
    let q = url.split_once('?')?.1;
    let get = |k: &str| q.split('&').find_map(|p| p.strip_prefix(k)?.strip_prefix('='));
    let name = format!("{}-{}-{}-{chunk}", get("id")?, get("itag")?, get("clen").unwrap_or("0"));
    name.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)).then(|| root().join(name))
}

fn expected(idx: u64, chunk: u64, total: u64) -> u64 {
    let (s, e) = crate::audio::chunk_range(idx * chunk, chunk, Some(total));
    e + 1 - s
}

/// The cached chunk `idx` and the stream's total length, if both are on disk and the chunk is whole.
pub fn read(url: &str, chunk: u64, idx: u64) -> Option<(Vec<u8>, u64)> {
    if LIMIT.load(Relaxed) == 0 {
        return None;
    }
    let d = dir(url, chunk)?;
    let total: u64 = fs::read_to_string(d.join("total")).ok()?.trim().parse().ok()?;
    let f = d.join(idx.to_string());
    let body = fs::read(&f).ok().filter(|b| b.len() as u64 == expected(idx, chunk, total))?;
    if let Ok(h) = fs::File::options().write(true).open(&f) {
        let _ = h.set_modified(SystemTime::now());
    }
    Some((body, total))
}

pub fn write(url: &str, chunk: u64, idx: u64, total: Option<u64>, body: &[u8]) {
    let limit = LIMIT.load(Relaxed);
    let (Some(total), Some(d)) = (total, dir(url, chunk)) else { return };
    if limit == 0 || body.len() as u64 != expected(idx, chunk, total) || fs::create_dir_all(&d).is_err() {
        return;
    }
    let tmp = d.join(format!("{idx}.part"));
    if fs::write(d.join("total"), total.to_string()).is_ok() && fs::write(&tmp, body).is_ok() && fs::rename(&tmp, d.join(idx.to_string())).is_ok() {
        trim(limit);
    }
}

fn files(dir: &Path, out: &mut Vec<(PathBuf, SystemTime, u64)>) {
    for e in fs::read_dir(dir).into_iter().flatten().flatten() {
        let (p, Ok(m)) = (e.path(), e.metadata()) else { continue };
        if m.is_dir() {
            files(&p, out);
        } else {
            out.push((p, m.modified().unwrap_or(SystemTime::UNIX_EPOCH), m.len()));
        }
    }
}

/// ponytail: rescans the tree on every write (a chunk is ~1 MB, so at most a few hundred files); index it if the limit grows to tens of GB.
fn trim(limit: u64) {
    let mut all = vec![];
    files(&root(), &mut all);
    let mut sum: u64 = all.iter().map(|f| f.2).sum();
    all.sort_by_key(|f| f.1);
    for (p, _, len) in all {
        if sum <= limit {
            break;
        }
        if fs::remove_file(&p).is_ok() {
            sum -= len;
        }
    }
    *USAGE.lock().unwrap() = None;
}

/// Bytes on disk, rescanned at most every two seconds (it is asked for each frame the settings page is open).
pub fn usage() -> u64 {
    let mut g = USAGE.lock().unwrap();
    if let Some((at, n)) = *g {
        if at.elapsed() < Duration::from_secs(2) {
            return n;
        }
    }
    let mut all = vec![];
    files(&root(), &mut all);
    let n = all.iter().map(|f| f.2).sum();
    *g = Some((Instant::now(), n));
    n
}

pub fn clear() {
    let _ = fs::remove_dir_all(root());
    *USAGE.lock().unwrap() = None;
}

#[cfg(test)]
mod tests {
    #[test]
    fn roundtrip_trim_and_clear() {
        let tmp = std::env::temp_dir().join(format!("serafin-cache-test-{}", std::process::id()));
        std::env::set_var("METROLIST_DATA", &tmp);
        let url = "https://x.googlevideo.com/videoplayback?expire=1&id=o-AB&itag=251&clen=2500";
        super::set_limit_mb(1);
        assert!(super::read(url, 1000, 0).is_none());
        super::write(url, 1000, 0, Some(2500), &[1; 1000]);
        super::write(url, 1000, 2, Some(2500), &[2; 500]);
        super::write(url, 1000, 1, Some(2500), &[3; 7]); // wrong length: refused
        assert_eq!(super::read(url, 1000, 0).unwrap(), (vec![1; 1000], 2500));
        assert_eq!(super::read(url, 1000, 2).unwrap().0.len(), 500);
        assert!(super::read(url, 1000, 1).is_none());
        assert!(super::dir("https://x/y?nope=1", 1).is_none());
        super::trim(1100); // drops the least recently read
        assert!(super::usage() <= 1100 + 8);
        super::clear();
        assert_eq!(super::usage(), 0);
        super::set_limit_mb(0);
        let _ = std::fs::remove_dir_all(tmp);
    }
}
