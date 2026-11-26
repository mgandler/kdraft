//! Content-addressed cache for `kicad-cli` outputs.
//!
//! Key: blake3 over (input content hash, `kicad-cli` version string, the
//! job's path-free arguments), each length-prefixed. Layout:
//!
//! ```text
//! <root>/<k0k1>/<key hex>/        one entry; job outputs inside
//! <root>/<k0k1>/<key hex>/KEY     human-readable description of the key
//! <root>/tmp/<unique>/            entries under construction
//! ```
//!
//! An entry is written in `tmp/` and renamed into place, so a visible entry
//! is always complete.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::SourceError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CacheKey(pub [u8; 32]);

impl CacheKey {
    pub fn new(input_hash: &[u8; 32], cli_version: &str, args: &[String]) -> CacheKey {
        let mut h = blake3::Hasher::new();
        h.update(b"kdraft-cache-v1");
        h.update(input_hash);
        let mut field = |b: &[u8]| {
            h.update(&(b.len() as u64).to_le_bytes());
            h.update(b);
        };
        field(cli_version.as_bytes());
        field(&(args.len() as u64).to_le_bytes());
        for a in args {
            field(a.as_bytes());
        }
        CacheKey(*h.finalize().as_bytes())
    }

    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }
}

#[derive(Debug, Clone)]
pub struct Cache {
    root: PathBuf,
}

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

impl Cache {
    pub fn new(root: impl Into<PathBuf>) -> Cache {
        Cache { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn entry_dir(&self, key: &CacheKey) -> PathBuf {
        let hex = key.to_hex();
        self.root.join(&hex[..2]).join(hex)
    }

    /// The entry directory if the key is cached.
    pub fn get(&self, key: &CacheKey) -> Option<PathBuf> {
        let d = self.entry_dir(key);
        d.is_dir().then_some(d)
    }

    /// Creates an entry: `produce` fills a fresh directory, which is then
    /// moved into place. If another writer finished first, its entry wins.
    /// On error nothing is cached.
    pub fn put(
        &self,
        key: &CacheKey,
        description: &str,
        produce: impl FnOnce(&Path) -> Result<(), SourceError>,
    ) -> Result<PathBuf, SourceError> {
        let final_dir = self.entry_dir(key);
        if final_dir.is_dir() {
            return Ok(final_dir);
        }
        let tmp_root = self.root.join("tmp");
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let tmp = tmp_root.join(format!(
            "{}-{}-{}",
            std::process::id(),
            nanos,
            TMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&tmp).map_err(|e| SourceError::io("creating", &tmp, e))?;
        let result = produce(&tmp).and_then(|()| {
            std::fs::write(tmp.join("KEY"), description)
                .map_err(|e| SourceError::io("writing", tmp.join("KEY"), e))
        });
        if let Err(e) = result {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(e);
        }
        let parent = final_dir.parent().unwrap_or(&self.root).to_path_buf();
        std::fs::create_dir_all(&parent).map_err(|e| SourceError::io("creating", &parent, e))?;
        match std::fs::rename(&tmp, &final_dir) {
            Ok(()) => Ok(final_dir),
            Err(_) if final_dir.is_dir() => {
                let _ = std::fs::remove_dir_all(&tmp);
                Ok(final_dir)
            }
            Err(e) => {
                let _ = std::fs::remove_dir_all(&tmp);
                Err(SourceError::io("moving cache entry to", &final_dir, e))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn key_is_deterministic_and_sensitive() {
        let h = [7u8; 32];
        let k = CacheKey::new(&h, "9.0.6", &args(&["a", "b"]));
        assert_eq!(k, CacheKey::new(&h, "9.0.6", &args(&["a", "b"])));
        assert_ne!(k, CacheKey::new(&[8u8; 32], "9.0.6", &args(&["a", "b"])));
        assert_ne!(k, CacheKey::new(&h, "9.0.7", &args(&["a", "b"])));
        assert_ne!(k, CacheKey::new(&h, "9.0.6", &args(&["b", "a"])));
        // Length prefixes keep field boundaries.
        assert_ne!(k, CacheKey::new(&h, "9.0.6", &args(&["ab"])));
        assert_ne!(
            CacheKey::new(&h, "9.0.6a", &args(&[])),
            CacheKey::new(&h, "9.0.6", &args(&["a"]))
        );
        assert_eq!(k.to_hex().len(), 64);
    }

    #[test]
    fn put_then_get() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path());
        let key = CacheKey::new(&[1; 32], "10.0.0", &args(&["x"]));
        assert_eq!(cache.get(&key), None);
        let entry = cache
            .put(&key, "test", |d| {
                std::fs::write(d.join("out.svg"), "<svg/>").map_err(|e| SourceError::io("w", d, e))
            })
            .unwrap();
        assert_eq!(cache.get(&key), Some(entry.clone()));
        assert_eq!(
            std::fs::read_to_string(entry.join("out.svg")).unwrap(),
            "<svg/>"
        );
        assert_eq!(std::fs::read_to_string(entry.join("KEY")).unwrap(), "test");
        assert!(entry.starts_with(dir.path().join(&key.to_hex()[..2])));
        // A second put does not run the producer.
        let again = cache.put(&key, "test", |_| panic!("must not run")).unwrap();
        assert_eq!(again, entry);
    }

    #[test]
    fn failed_put_caches_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path());
        let key = CacheKey::new(&[2; 32], "10.0.0", &[]);
        let e = cache.put(&key, "x", |_| Err(SourceError::MissingOutput("o".into())));
        assert!(e.is_err());
        assert_eq!(cache.get(&key), None);
        let tmp_entries = std::fs::read_dir(dir.path().join("tmp")).unwrap().count();
        assert_eq!(tmp_entries, 0);
    }
}
