// mind init — workspace initialization wizard (MF-002)

use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use crate::infra::atomic;
use crate::model::config::WorkspaceConfig;

/// Errors returned by the init wizard.
#[derive(Debug)]
pub enum InitError {
    Io(io::Error),
    DirectoryNotFound(PathBuf),
    NotWritable(PathBuf),
    Aborted,
}

impl std::fmt::Display for InitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InitError::Io(e) => write!(f, "I/O error: {}", e),
            InitError::DirectoryNotFound(p) => {
                write!(f, "Directory not found: {}", p.display())
            }
            InitError::NotWritable(p) => {
                write!(f, "Cannot write to {} — check permissions", p.display())
            }
            InitError::Aborted => write!(f, "Aborted."),
        }
    }
}

impl From<io::Error> for InitError {
    fn from(e: io::Error) -> Self {
        InitError::Io(e)
    }
}

/// Check if a directory contains a `.git` subdirectory (i.e., is a git repo).
fn is_git_repo(dir: &Path) -> bool {
    dir.join(".git").exists()
}

/// Return sorted list of immediate subdirectories that are git repos.
fn git_repo_subdirs(dir: &Path) -> Vec<PathBuf> {
    let mut repos = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && is_git_repo(&path) {
                repos.push(path);
            }
        }
    }
    repos.sort();
    repos
}

/// Count immediate subdirectories of `dir` that are git repos.
fn count_git_repos(dir: &Path) -> usize {
    git_repo_subdirs(dir).len()
}

/// Walk up from `start` to find a directory that:
/// - Is NOT itself a git repo (no `.git` in it)
/// - Contains at least one subdirectory that IS a git repo
///
/// Falls back to `start` if no such directory is found.
/// Algorithm from 03-COMMANDS.md §Default detection.
fn detect_candidate(start: &Path) -> PathBuf {
    let mut current = start.to_path_buf();
    loop {
        if !is_git_repo(&current) && count_git_repos(&current) > 0 {
            return current;
        }
        match current.parent() {
            Some(p) if p != current => current = p.to_path_buf(),
            _ => return start.to_path_buf(),
        }
    }
}

/// Read one line from `reader`, stripping the trailing newline.
fn prompt_line<R: BufRead>(reader: &mut R) -> Result<String, io::Error> {
    let mut line = String::new();
    reader.read_line(&mut line)?;
    Ok(line
        .trim_end_matches('\n')
        .trim_end_matches('\r')
        .to_string())
}

/// Entry point for `mind init`. Detects workspace candidate from cwd.
pub fn run() -> Result<(), InitError> {
    let start = std::env::current_dir()?;
    let stdin = io::stdin();
    let stdout = io::stdout();
    run_with_io(&start, stdin.lock(), stdout.lock())
}

/// Inner wizard — accepts injectable reader and writer for testability.
///
/// `start` is the directory from which candidate detection begins (usually cwd).
pub(crate) fn run_with_io<R: BufRead, W: Write>(
    start: &Path,
    mut reader: R,
    mut writer: W,
) -> Result<(), InitError> {
    // Phase 2 — Detect candidate workspace root
    let candidate = detect_candidate(start);
    let n_repos = count_git_repos(&candidate);

    writeln!(writer)?;
    writeln!(writer, "Detecting workspace root...")?;
    writeln!(
        writer,
        "  Candidate: {}  (contains {} git repos)",
        candidate.display(),
        n_repos
    )?;
    writeln!(writer)?;
    write!(writer, "Workspace root [{}]: ", candidate.display())?;
    writer.flush()?;

    // Read user input — empty means accept default
    let input = prompt_line(&mut reader)?;
    let chosen = if input.is_empty() {
        candidate.clone()
    } else {
        PathBuf::from(&input)
    };

    // Validate path exists
    if !chosen.exists() {
        return Err(InitError::DirectoryNotFound(chosen));
    }

    // Check writability (attempt metadata read; actual write errors surface during init)
    if std::fs::metadata(&chosen)
        .map(|m| m.permissions().readonly())
        .unwrap_or(true)
    {
        return Err(InitError::NotWritable(chosen));
    }

    // Show git repos at chosen path
    let repos = git_repo_subdirs(&chosen);
    let repo_count = repos.len();

    writeln!(writer, "→ {}", chosen.display())?;
    let display_count = repo_count.min(4);
    for repo in repos.iter().take(display_count) {
        let name = repo.file_name().unwrap_or_default().to_string_lossy();
        writeln!(writer, "    {}/  \u{2713} git repo", name)?;
    }
    if repo_count > 4 {
        writeln!(writer, "    ... and {} more", repo_count - 4)?;
    }
    writeln!(writer)?;

    // No git repos warning — not a hard error, but warn and confirm
    if repo_count == 0 {
        write!(writer, "No git repos found here. Continue anyway? [y/N]: ")?;
        writer.flush()?;
        let response = prompt_line(&mut reader)?;
        if !response.eq_ignore_ascii_case("y") {
            return Err(InitError::Aborted);
        }
    }

    // Check if already initialized at chosen path
    let mind_root_path = chosen.join(".mind-root");
    if mind_root_path.exists() {
        write!(
            writer,
            "Already initialized at {}. Reinitialize? [y/N]: ",
            chosen.display()
        )?;
        writer.flush()?;
        let response = prompt_line(&mut reader)?;
        if !response.eq_ignore_ascii_case("y") {
            return Err(InitError::Aborted);
        }
        return do_reinit(&chosen, &mut writer);
    }

    // Confirm placement
    write!(writer, "Place .mind-root here? [Y/n]: ")?;
    writer.flush()?;
    let response = prompt_line(&mut reader)?;
    if response.eq_ignore_ascii_case("n") {
        return Err(InitError::Aborted);
    }

    do_init(&chosen, &mut writer)
}

/// Default `.mindacked` content written by `mind init`.
///
/// Contains a comment header explaining the syntax and purpose.
/// No patterns are included by default — the list starts empty.
/// Workspace-specific paths are NOT included as defaults — users add their own.
///
/// Note: `.mindignore` (exclude from index) and `.mindacked` (suppress output) are
/// orthogonal. A path in both is valid; `.mindignore` wins (not scanned = no refs).
/// Adding the same path to `.mindacked` is harmless but redundant.
const DEFAULT_MINDACKED: &str = "\
# .mindacked — acknowledged broken references (deferred repair)
# Source paths matching these patterns will have their broken outbound refs
# suppressed from `mind file validate` output.
# Syntax follows .gitignore rules (https://git-scm.com/docs/gitignore)
# A pattern without / matches at any depth. /pattern anchors to workspace root.
#
# Files matching these patterns remain fully indexed — they are still valid
# reference targets. This list represents repair debt — review it periodically.
# Note: .mindignore (exclude from index) and .mindacked (suppress output) are
# orthogonal. A path in both is valid; .mindignore wins (not scanned = no refs).
";

/// Default `.mindignore` content written by `mind init`.
///
/// Contains sensible defaults (node_modules/, target/).
/// Includes a comment header explaining the syntax and Phase 1 scope.
/// Workspace-specific paths are NOT included as defaults — users add their own.
const DEFAULT_MINDIGNORE: &str = "\
# .mindignore — patterns excluded from mind file operations
# Syntax follows .gitignore rules (https://git-scm.com/docs/gitignore)
# A pattern without / matches at any depth. /pattern anchors to workspace root.
# Note: per-directory .mindignore files are not supported in Phase 1 — only
# the root-level file is read.

# Third-party package directories
node_modules/

# Build artifacts
target/
";

/// Write `.mindignore` at `path` if it does not already exist.
///
/// Returns `true` if the file was created, `false` if it already existed and was skipped.
/// Never overwrites an existing `.mindignore` — users may have customized it.
fn ensure_mindignore(path: &Path) -> Result<bool, InitError> {
    let mindignore_path = path.join(".mindignore");
    if mindignore_path.exists() {
        Ok(false)
    } else {
        std::fs::write(&mindignore_path, DEFAULT_MINDIGNORE)?;
        Ok(true)
    }
}

/// Write `.mindacked` at `path` if it does not already exist.
///
/// Returns `true` if the file was created, `false` if it already existed and was skipped.
/// Never overwrites an existing `.mindacked` — users may have customized it.
fn ensure_mindacked(path: &Path) -> Result<bool, InitError> {
    let mindacked_path = path.join(".mindacked");
    if mindacked_path.exists() {
        Ok(false)
    } else {
        std::fs::write(&mindacked_path, DEFAULT_MINDACKED)?;
        Ok(true)
    }
}

/// Execute fresh initialization at `path`.
fn do_init<W: Write>(path: &Path, writer: &mut W) -> Result<(), InitError> {
    // a. Create .mind-root as empty file (zero bytes)
    std::fs::write(path.join(".mind-root"), b"")?;

    // b. Create .mind/ directory
    std::fs::create_dir_all(path.join(".mind"))?;

    // c. Write .mind/config.json atomically
    let config = WorkspaceConfig::phase1();
    let config_json =
        serde_json::to_string(&config).expect("WorkspaceConfig serialization is infallible");
    let config_path = path.join(".mind").join("config.json");
    atomic::atomic_write(&config_path, &config_json).map_err(|e| InitError::Io(e.into()))?;

    // d. Write .mindignore at workspace root (only if not already present)
    let mindignore_created = ensure_mindignore(path)?;

    // e. Write .mindacked at workspace root (only if not already present)
    let mindacked_created = ensure_mindacked(path)?;

    // f. Confirmation output
    writeln!(writer, "\u{2192} Created  .mind-root")?;
    writeln!(
        writer,
        "\u{2192} Created  .mind/config.json  {{\"schema_version\": \"1\"}}"
    )?;
    if mindignore_created {
        writeln!(writer, "\u{2192} Created  .mindignore")?;
    } else {
        writeln!(writer, "\u{2192} Skipped  .mindignore (already exists)")?;
    }
    if mindacked_created {
        writeln!(writer, "\u{2192} Created  .mindacked")?;
    } else {
        writeln!(writer, "\u{2192} Skipped  .mindacked (already exists)")?;
    }
    writeln!(writer)?;
    writeln!(writer, "Done. Workspace root: {}", path.display())?;
    writeln!(
        writer,
        "Next: run 'mind file validate' to check reference health."
    )?;

    Ok(())
}

/// Execute re-initialization at `path` — overwrite config.json only.
///
/// PHASE-2-BRIDGE Contract 3 guard: never touch knowledge.db
/// even though it doesn't exist in Phase 1.
/// Re-init writes config.json only. All other .mind/ contents (including
/// knowledge.db when it exists in Phase 2) are explicitly preserved.
fn do_reinit<W: Write>(path: &Path, writer: &mut W) -> Result<(), InitError> {
    let mind_dir = path.join(".mind");
    std::fs::create_dir_all(&mind_dir)?;

    // PHASE-2-BRIDGE Contract 3 guard: never touch knowledge.db
    // even though it doesn't exist in Phase 1.
    // The only file we write in re-init is config.json.
    // All other .mind/ contents — including knowledge.db — are not touched.

    let config = WorkspaceConfig::phase1();
    let config_json =
        serde_json::to_string(&config).expect("WorkspaceConfig serialization is infallible");
    let config_path = mind_dir.join("config.json");
    atomic::atomic_write(&config_path, &config_json).map_err(|e| InitError::Io(e.into()))?;

    // Write .mindignore if not present (safe to add to existing workspaces)
    let mindignore_created = ensure_mindignore(path)?;

    // Write .mindacked if not present (safe to add to existing workspaces)
    let mindacked_created = ensure_mindacked(path)?;

    writeln!(
        writer,
        "\u{2192} Created  .mind/config.json  {{\"schema_version\": \"1\"}}"
    )?;
    if mindignore_created {
        writeln!(writer, "\u{2192} Created  .mindignore")?;
    } else {
        writeln!(writer, "\u{2192} Skipped  .mindignore (already exists)")?;
    }
    if mindacked_created {
        writeln!(writer, "\u{2192} Created  .mindacked")?;
    } else {
        writeln!(writer, "\u{2192} Skipped  .mindacked (already exists)")?;
    }
    writeln!(writer)?;
    writeln!(writer, "Done. Workspace root: {}", path.display())?;
    writeln!(
        writer,
        "Next: run 'mind file validate' to check reference health."
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn make_git_repo(dir: &Path) {
        fs::create_dir_all(dir.join(".git")).unwrap();
    }

    /// Happy path: temp dir with two git repo subdirs (but no .git itself).
    /// Verifies: .mind-root is zero bytes, config.json deserializes to schema_version "1",
    /// no .tmp file left behind.
    #[test]
    fn test_happy_path() {
        let root = tempfile::tempdir().unwrap();
        make_git_repo(&root.path().join("repo-a"));
        make_git_repo(&root.path().join("repo-b"));

        // detect_candidate(root) → root (not a git repo, contains 2 git repo subdirs)
        // Input: accept default root (Enter), accept "Place .mind-root here?" (Enter)
        let input = "\n\n";
        let mut output = Vec::new();

        run_with_io(root.path(), input.as_bytes(), &mut output).unwrap();

        // .mind-root exists at root and is zero bytes
        let mind_root = root.path().join(".mind-root");
        assert!(mind_root.exists(), ".mind-root must exist");
        assert_eq!(
            fs::metadata(&mind_root).unwrap().len(),
            0,
            ".mind-root must be zero bytes"
        );

        // .mind/config.json exists and deserializes correctly
        let config_path = root.path().join(".mind").join("config.json");
        assert!(config_path.exists(), ".mind/config.json must exist");
        let content = fs::read_to_string(&config_path).unwrap();
        let config: WorkspaceConfig = serde_json::from_str(&content).unwrap();
        assert_eq!(
            config.schema_version, "1",
            "schema_version must be string '1'"
        );

        // Atomic write: no .tmp file left behind
        let tmp_path = root.path().join(".mind").join("config.json.tmp");
        assert!(
            !tmp_path.exists(),
            ".tmp file must not be left behind after init"
        );
    }

    /// Re-init: existing .mind-root + config.json. Simulate confirming re-init.
    /// Verifies: only config.json overwritten, knowledge.db and other .mind/ contents untouched.
    #[test]
    fn test_reinit() {
        let root = tempfile::tempdir().unwrap();
        make_git_repo(&root.path().join("repo-a"));

        // Pre-existing initialization
        fs::write(root.path().join(".mind-root"), b"").unwrap();
        fs::create_dir_all(root.path().join(".mind")).unwrap();
        fs::write(
            root.path().join(".mind").join("config.json"),
            r#"{"schema_version":"1"}"#,
        )
        .unwrap();

        // Fake knowledge.db — must NOT be touched (PHASE-2-BRIDGE Contract 3 guard)
        let knowledge_db_path = root.path().join(".mind").join("knowledge.db");
        let original_db_content = b"fake knowledge db content";
        fs::write(&knowledge_db_path, original_db_content).unwrap();

        // Other .mind/ content — must also be preserved
        let other_file = root.path().join(".mind").join("other.txt");
        fs::write(&other_file, b"preserved").unwrap();

        // Input: accept default root (Enter), confirm re-init with "y"
        let input = "\ny\n";
        let mut output = Vec::new();

        run_with_io(root.path(), input.as_bytes(), &mut output).unwrap();

        // config.json was overwritten with valid content
        let config_path = root.path().join(".mind").join("config.json");
        let content = fs::read_to_string(&config_path).unwrap();
        let config: WorkspaceConfig = serde_json::from_str(&content).unwrap();
        assert_eq!(config.schema_version, "1");

        // knowledge.db NOT touched (PHASE-2-BRIDGE Contract 3 guard code path verified)
        assert!(knowledge_db_path.exists(), "knowledge.db must still exist");
        assert_eq!(
            fs::read(&knowledge_db_path).unwrap(),
            original_db_content,
            "knowledge.db content must be unchanged"
        );

        // other.txt NOT touched
        assert!(other_file.exists(), "other.txt must still exist");
        assert_eq!(fs::read(&other_file).unwrap(), b"preserved");
    }

    /// Path not found: user supplies a non-existent directory.
    /// Verifies: returns error containing "Directory not found: ".
    #[test]
    fn test_path_not_found() {
        let start = std::env::current_dir().unwrap();
        let nonexistent = "/nonexistent/path/that/does/not/exist/9f3k2j1";

        // Input: type non-existent path at workspace root prompt
        let input = format!("{}\n", nonexistent);
        let mut output = Vec::new();

        let result = run_with_io(&start, input.as_bytes(), &mut output);

        match result {
            Err(InitError::DirectoryNotFound(p)) => {
                let msg = format!("Directory not found: {}", p.display());
                assert!(
                    msg.contains("Directory not found: "),
                    "error must contain 'Directory not found: ', got: {}",
                    msg
                );
            }
            other => panic!("expected DirectoryNotFound, got: {:?}", other),
        }
    }

    /// Already initialized, user declines: exits cleanly with no changes.
    /// Verifies: returns Aborted, workspace unchanged.
    #[test]
    fn test_decline_reinit() {
        let root = tempfile::tempdir().unwrap();
        make_git_repo(&root.path().join("repo-a"));

        // Pre-existing .mind-root + config.json
        fs::write(root.path().join(".mind-root"), b"").unwrap();
        fs::create_dir_all(root.path().join(".mind")).unwrap();
        let original_config = r#"{"schema_version":"1"}"#;
        fs::write(
            root.path().join(".mind").join("config.json"),
            original_config.as_bytes(),
        )
        .unwrap();

        // Input: accept default root (Enter), decline re-init with "n"
        let input = "\nn\n";
        let mut output = Vec::new();

        let result = run_with_io(root.path(), input.as_bytes(), &mut output);

        assert!(
            matches!(result, Err(InitError::Aborted)),
            "expected Aborted, got: {:?}",
            result
        );

        // config.json unchanged
        let content = fs::read_to_string(root.path().join(".mind").join("config.json")).unwrap();
        assert_eq!(
            content, original_config,
            "config.json must be unchanged after declining"
        );
    }
}
