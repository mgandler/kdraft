//! Board file change detection by polling size and modification time
//! (no extra dependency, behaves the same on every platform). A change is
//! reported once the stamp has been stable for one poll, so a file that
//! is still being written is not read half-way.

use std::path::Path;
use std::time::SystemTime;

/// Poll interval, milliseconds.
pub const POLL_MS: u64 = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    pub len: u64,
    pub modified: Option<SystemTime>,
}

pub fn stamp(path: &Path) -> Option<Stamp> {
    let m = std::fs::metadata(path).ok()?;
    Some(Stamp {
        len: m.len(),
        modified: m.modified().ok(),
    })
}

/// Debounced change detector for one file.
#[derive(Debug, Clone, Default)]
pub struct Watch {
    /// Stamp of the version the app has loaded.
    loaded: Option<Stamp>,
    /// Last stamp seen, if it differs from `loaded`.
    candidate: Option<Option<Stamp>>,
}

impl Watch {
    pub fn new(loaded: Option<Stamp>) -> Watch {
        Watch {
            loaded,
            candidate: None,
        }
    }

    /// Feeds one poll; true when a change is confirmed (and taken as the
    /// new loaded state). A missing file is a change too.
    pub fn observe(&mut self, now: Option<Stamp>) -> bool {
        if now == self.loaded {
            self.candidate = None;
            return false;
        }
        match self.candidate {
            Some(c) if c == now => {
                self.loaded = now;
                self.candidate = None;
                true
            }
            _ => {
                self.candidate = Some(now);
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn st(len: u64, s: u64) -> Option<Stamp> {
        Some(Stamp {
            len,
            modified: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(s)),
        })
    }

    #[test]
    fn reports_after_one_stable_poll() {
        let mut w = Watch::new(st(10, 1));
        assert!(!w.observe(st(10, 1)));
        assert!(!w.observe(st(20, 2)));
        // Still growing.
        assert!(!w.observe(st(30, 3)));
        assert!(w.observe(st(30, 3)));
        assert!(!w.observe(st(30, 3)));
        // Deleted, then stays deleted.
        assert!(!w.observe(None));
        assert!(w.observe(None));
    }

    #[test]
    fn real_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("b.kicad_pcb");
        std::fs::write(&p, "a").unwrap();
        let mut w = Watch::new(stamp(&p));
        std::fs::write(&p, "abc").unwrap();
        assert!(!w.observe(stamp(&p)));
        assert!(w.observe(stamp(&p)));
    }
}
