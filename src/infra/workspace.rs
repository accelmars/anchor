use std::io;
use std::path::PathBuf;

/// Errors returned by workspace discovery.
#[derive(Debug)]
pub enum WorkspaceError {
    /// No `.mind-root` marker was found anywhere up the directory tree.
    NotFound,
    /// An I/O error occurred while traversing the filesystem.
    IoError(io::Error),
}

impl std::fmt::Display for WorkspaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WorkspaceError::NotFound => {
                write!(f, "no workspace found. Run 'mind init' to configure.")
            }
            WorkspaceError::IoError(e) => write!(f, "I/O error: {}", e),
        }
    }
}

impl From<io::Error> for WorkspaceError {
    fn from(e: io::Error) -> Self {
        WorkspaceError::IoError(e)
    }
}

/// Walk up the directory tree from the current working directory, looking for a
/// `.mind-root` marker file. Returns the path of the directory containing the
/// marker, with no trailing slash.
///
/// Algorithm (verbatim from 02-WORKSPACE.md §Root Discovery Algorithm):
/// 1. Start at cwd
/// 2. Check if .mind-root exists in current directory
/// 3. If yes → workspace root found, return this path
/// 4. If no → move to parent directory
/// 5. If reached filesystem root (/) with no .mind-root found:
///    → hard error: "no workspace found. Run 'mind init' to configure."
/// 6. Repeat from step 2
pub fn find_workspace_root() -> Result<PathBuf, WorkspaceError> {
    let mut current = std::env::current_dir().map_err(WorkspaceError::IoError)?;

    loop {
        let marker = current.join(".mind-root");
        match marker.exists() {
            true => return Ok(current),
            false => {
                let parent = current.parent().map(|p| p.to_path_buf());
                match parent {
                    Some(p) if p != current => {
                        current = p;
                    }
                    _ => return Err(WorkspaceError::NotFound),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Happy path: `.mind-root` exists in the current directory.
    /// Verifies: returns Ok(path) matching the temp dir, no trailing slash.
    #[test]
    fn test_found_happy_path() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let marker = dir.path().join(".mind-root");
        fs::write(&marker, "").expect("failed to write marker");

        // Change cwd to the temp dir, run discovery, restore cwd.
        let original = std::env::current_dir().expect("failed to get cwd");
        std::env::set_current_dir(dir.path()).expect("failed to set cwd");

        let result = find_workspace_root();

        std::env::set_current_dir(&original).expect("failed to restore cwd");

        let root = result.expect("should find workspace root");
        // No trailing slash
        let as_str = root.to_string_lossy();
        assert!(
            !as_str.ends_with('/'),
            "root path must not have a trailing slash, got: {}",
            as_str
        );
        // Path must match the temp dir (canonicalized)
        assert_eq!(
            root.canonicalize().expect("canonicalize root"),
            dir.path().canonicalize().expect("canonicalize dir")
        );
    }

    /// Not found: no `.mind-root` anywhere up the tree from a fresh temp directory.
    /// Verifies: returns Err(WorkspaceError::NotFound), does not loop infinitely.
    #[test]
    fn test_not_found_filesystem_root_stop() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        // No .mind-root anywhere up this tree (assuming /tmp or equivalent has none).

        let original = std::env::current_dir().expect("failed to get cwd");
        std::env::set_current_dir(dir.path()).expect("failed to set cwd");

        let result = find_workspace_root();

        std::env::set_current_dir(&original).expect("failed to restore cwd");

        match result {
            Err(WorkspaceError::NotFound) => {}
            other => panic!("expected NotFound, got: {:?}", other),
        }
    }

    /// Nested ascent: `.mind-root` at root of temp dir, cwd is a deep subdirectory.
    /// Verifies: walk-up finds `.mind-root` at the correct root level.
    #[test]
    fn test_nested_subdirectory_ascent() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let marker = dir.path().join(".mind-root");
        fs::write(&marker, "").expect("failed to write marker");

        let deep = dir.path().join("2").join("3").join("4");
        fs::create_dir_all(&deep).expect("failed to create nested dirs");

        let original = std::env::current_dir().expect("failed to get cwd");
        std::env::set_current_dir(&deep).expect("failed to set cwd to deep dir");

        let result = find_workspace_root();

        std::env::set_current_dir(&original).expect("failed to restore cwd");

        let root = result.expect("should find workspace root via ascent");
        assert_eq!(
            root.canonicalize().expect("canonicalize root"),
            dir.path().canonicalize().expect("canonicalize dir"),
            "walk-up should find .mind-root at the workspace root, not at the deep subdirectory"
        );
    }
}
