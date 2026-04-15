// Atomic file write — write-to-tmp + rename (MF-002/MF-005)
//
// PHASE-2-BRIDGE Contract 6: config.json is written atomically.
// Algorithm: write content to {path}.tmp, then rename to {path}.
// This prevents a corrupted config from a partial write (power loss during mind init).

use std::io;
use std::path::Path;

/// Error returned by atomic_write.
#[derive(Debug)]
pub struct AtomicWriteError(pub io::Error);

impl std::fmt::Display for AtomicWriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "atomic write failed: {}", self.0)
    }
}

impl From<AtomicWriteError> for io::Error {
    fn from(e: AtomicWriteError) -> Self {
        e.0
    }
}

/// Write `content` to `path` atomically using write-to-tmp + rename.
///
/// Algorithm (from 02-WORKSPACE.md §Writing rules, Phase 2 Bridge Contract 6):
/// 1. Compute tmp path: `{path}.tmp` (same directory, same filesystem)
/// 2. Write `content` to `{path}.tmp`
/// 3. `std::fs::rename({path}.tmp, path)` — atomic on POSIX same-filesystem
/// 4. Return `Ok(())`
///
/// If any step fails, returns `AtomicWriteError` wrapping the underlying `io::Error`.
pub fn atomic_write(path: &Path, content: &str) -> Result<(), AtomicWriteError> {
    let tmp_path = {
        let mut p = path.as_os_str().to_owned();
        p.push(".tmp");
        std::path::PathBuf::from(p)
    };

    std::fs::write(&tmp_path, content).map_err(AtomicWriteError)?;
    std::fs::rename(&tmp_path, path).map_err(AtomicWriteError)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_atomic_write_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");

        atomic_write(&path, r#"{"schema_version":"1"}"#).unwrap();

        assert!(path.exists());
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, r#"{"schema_version":"1"}"#);
    }

    #[test]
    fn test_atomic_write_no_tmp_leftover() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let tmp_path = dir.path().join("config.json.tmp");

        atomic_write(&path, r#"{"schema_version":"1"}"#).unwrap();

        assert!(!tmp_path.exists(), ".tmp file must not be left behind");
    }

    #[test]
    fn test_atomic_write_overwrites_existing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");

        std::fs::write(&path, "old content").unwrap();
        atomic_write(&path, "new content").unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "new content");
    }
}
