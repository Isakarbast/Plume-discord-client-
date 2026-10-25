//! Downloaded files on disk, named by a hash of their URL.

use std::fs;
use std::path::PathBuf;
use std::time::SystemTime;

use tracing::warn;

pub struct DiskCache {
    dir: PathBuf,
    max_bytes: u64,
}

impl DiskCache {
    pub fn new(dir: PathBuf, max_bytes: u64) -> Self {
        let _ = fs::create_dir_all(&dir);
        DiskCache { dir, max_bytes }
    }

    pub fn read(&self, url: &str) -> Option<Vec<u8>> {
        fs::read(self.path_for(url)).ok()
    }

    /// Best effort: a full disk just means no caching.
    pub fn write(&self, url: &str, bytes: &[u8]) {
        let path = self.path_for(url);
        if path.exists() {
            return; // another download of the same image got here first
        }
        // Write to a temp file and rename, so a crash never leaves half a file.
        // Unique per write: the same image can be fetched twice at once.
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let temp = path.with_extension(format!("{}-{n}.part", std::process::id()));
        if fs::write(&temp, bytes)
            .and_then(|()| fs::rename(&temp, &path))
            .is_err()
        {
            let _ = fs::remove_file(&temp);
            warn!("couldn't write {} to the media cache", path.display());
        }
    }

    /// Deletes the oldest files until the cache fits its limit.
    pub fn prune(&self) {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return;
        };
        let mut files: Vec<(SystemTime, u64, PathBuf)> = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let meta = entry.metadata().ok()?;
                meta.is_file().then(|| {
                    (
                        meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                        meta.len(),
                        entry.path(),
                    )
                })
            })
            .collect();
        let mut total: u64 = files.iter().map(|(_, size, _)| size).sum();
        files.sort(); // oldest first
        for (_, size, path) in files {
            if total <= self.max_bytes {
                break;
            }
            if fs::remove_file(&path).is_ok() {
                total -= size;
            }
        }
    }

    fn path_for(&self, url: &str) -> PathBuf {
        self.dir.join(format!("{:016x}", fnv1a(url.as_bytes())))
    }
}

/// FNV-1a: a tiny hash that, unlike std's, gives the same answer forever,
/// so file names stay valid across Rust versions.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("plume-media-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn writes_reads_and_prunes() {
        let dir = temp_dir("disk");
        let cache = DiskCache::new(dir.clone(), 25);
        cache.write("https://a", &[1; 10]);
        std::thread::sleep(std::time::Duration::from_millis(20));
        cache.write("https://b", &[2; 10]);
        std::thread::sleep(std::time::Duration::from_millis(20));
        cache.write("https://c", &[3; 10]);
        assert_eq!(cache.read("https://b"), Some(vec![2; 10]));

        cache.prune(); // 30 bytes > 25: the oldest (a) goes
        assert_eq!(cache.read("https://a"), None);
        assert!(cache.read("https://c").is_some());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn hash_is_stable() {
        assert_eq!(fnv1a(b"hello"), 0xa430_d846_80aa_bd0b);
    }
}
