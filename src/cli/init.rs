// mind init — workspace initialization wizard (MF-002 / MX-003)

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
    NoCandidate,
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
            InitError::NoCandidate => {
                write!(f, "no workspace candidate detected — use --path to specify")
            }
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

/// Returns true if `dir` qualifies as a valid workspace candidate:
/// not itself a git repo, but contains at least one git repo subdirectory.
fn is_workspace_candidate(dir: &Path) -> bool {
    !is_git_repo(dir) && count_git_repos(dir) > 0
}

/// Validate that `path` exists and is writable.
fn validate_path(path: &Path) -> Result<(), InitError> {
    if !path.exists() {
        return Err(InitError::DirectoryNotFound(path.to_path_buf()));
    }
    if std::fs::metadata(path)
        .map(|m| m.permissions().readonly())
        .unwrap_or(true)
    {
        return Err(InitError::NotWritable(path.to_path_buf()));
    }
    Ok(())
}

/// Compute how many visible (prompted) steps the wizard will show.
///
/// `has_workspace_step` — true when the workspace root prompt will be shown.
/// `no_repos` — true when the "no repos, continue?" prompt will be shown.
/// `is_reinit` — true when the re-init path is taken (adds confirm + writing step).
fn compute_step_total(has_workspace_step: bool, no_repos: bool, is_reinit: bool) -> usize {
    let mut n = 0;
    if has_workspace_step {
        n += 1;
    }
    if no_repos {
        n += 1;
    }
    if is_reinit {
        n += 2; // re-init confirm + writing step
    } else {
        n += 1; // place confirm
    }
    n
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
///
/// `yes` — accept detected workspace root without prompting (fully non-interactive).
/// `path` — use given path instead of detection; skip detection entirely.
pub fn run(yes: bool, path: Option<&str>) -> Result<(), InitError> {
    let start = std::env::current_dir()?;
    let stdin = io::stdin();
    let stdout = io::stdout();
    run_with_io(&start, stdin.lock(), stdout.lock(), yes, path)
}

/// Inner wizard — accepts injectable reader and writer for testability.
///
/// `start` is the directory from which candidate detection begins (usually cwd).
/// `yes` — accept detected workspace root without prompting (fully non-interactive).
/// `explicit_path` — use given path instead of detection.
pub(crate) fn run_with_io<R: BufRead, W: Write>(
    start: &Path,
    mut reader: R,
    mut writer: W,
    yes: bool,
    explicit_path: Option<&str>,
) -> Result<(), InitError> {
    // ── Resolve workspace root ─────────────────────────────────────────
    let chosen: PathBuf;
    let step_total: usize;
    let mut step_current: usize = 1;

    if let Some(p) = explicit_path {
        // --path provided: validate immediately, skip detection entirely.
        let dir = PathBuf::from(p);
        validate_path(&dir)?;
        chosen = dir;
        if yes {
            // --yes --path: no prompts at all.
            step_total = 0;
        } else {
            // --path only: show remaining prompts, no workspace root step.
            let is_reinit = chosen.join(".mind-root").exists();
            let no_repos = count_git_repos(&chosen) == 0;
            step_total = compute_step_total(false, no_repos, is_reinit);
        }
    } else {
        let candidate = detect_candidate(start);
        let n_repos = count_git_repos(&candidate);

        if yes {
            // --yes: accept detected root, no output, no prompts.
            if !is_workspace_candidate(&candidate) {
                return Err(InitError::NoCandidate);
            }
            validate_path(&candidate)?;
            chosen = candidate;
            step_total = 0;
        } else {
            // Interactive wizard: show detection output and prompt.
            writeln!(writer)?;
            writeln!(writer, "Detecting workspace root...")?;
            writeln!(
                writer,
                "  Candidate: {}  (contains {} git repos)",
                candidate.display(),
                n_repos
            )?;
            writeln!(writer)?;

            let is_reinit_candidate = candidate.join(".mind-root").exists();
            let no_repos_candidate = n_repos == 0;
            step_total = compute_step_total(true, no_repos_candidate, is_reinit_candidate);

            // Step 1: Workspace root prompt.
            write!(
                writer,
                "[{}/{}] Workspace root [{}]: ",
                step_current,
                step_total,
                candidate.display()
            )?;
            writer.flush()?;
            step_current += 1;

            let input = prompt_line(&mut reader)?;
            if input.is_empty() {
                validate_path(&candidate)?;
                chosen = candidate;
            } else {
                let typed = PathBuf::from(&input);
                // Error retry (C28 pattern): one retry on path errors.
                if !typed.exists() {
                    writeln!(writer, "{}", InitError::DirectoryNotFound(typed.clone()))?;
                    write!(writer, "Enter a different path: ")?;
                    writer.flush()?;
                    let second = prompt_line(&mut reader)?;
                    let second_path = PathBuf::from(&second);
                    validate_path(&second_path)?;
                    chosen = second_path;
                } else if std::fs::metadata(&typed)
                    .map(|m| m.permissions().readonly())
                    .unwrap_or(true)
                {
                    writeln!(writer, "{}", InitError::NotWritable(typed.clone()))?;
                    write!(writer, "Enter a different path: ")?;
                    writer.flush()?;
                    let second = prompt_line(&mut reader)?;
                    let second_path = PathBuf::from(&second);
                    validate_path(&second_path)?;
                    chosen = second_path;
                } else {
                    chosen = typed;
                }
            }
        }
    }

    // ── Common path: chosen is resolved and validated ──────────────────

    let repos = git_repo_subdirs(&chosen);
    let repo_count = repos.len();

    // Show git repos at chosen path (interactive modes only — silent in --yes).
    if step_total > 0 {
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
    }

    // No git repos warning — not a hard error, but warn and confirm.
    if repo_count == 0 && step_total > 0 {
        write!(
            writer,
            "[{}/{}] No git repos found here. Continue anyway? [y/N]: ",
            step_current, step_total
        )?;
        writer.flush()?;
        let response = prompt_line(&mut reader)?;
        if !response.eq_ignore_ascii_case("y") {
            return Err(InitError::Aborted);
        }
        step_current += 1;
    }

    // Check if already initialized at chosen path.
    let mind_root_path = chosen.join(".mind-root");
    if mind_root_path.exists() {
        if step_total > 0 {
            write!(
                writer,
                "[{}/{}] Already initialized at {}. Reinitialize? [y/N]: ",
                step_current,
                step_total,
                chosen.display()
            )?;
            writer.flush()?;
            let response = prompt_line(&mut reader)?;
            if !response.eq_ignore_ascii_case("y") {
                return Err(InitError::Aborted);
            }
            step_current += 1;
            writeln!(
                writer,
                "[{}/{}] Writing workspace files...",
                step_current, step_total
            )?;
        }
        return do_reinit(&chosen, &mut writer);
    }

    // Confirm placement.
    if step_total > 0 {
        write!(
            writer,
            "[{}/{}] Place .mind-root here? [Y/n]: ",
            step_current, step_total
        )?;
        writer.flush()?;
        let response = prompt_line(&mut reader)?;
        if response.eq_ignore_ascii_case("n") {
            return Err(InitError::Aborted);
        }
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

        run_with_io(root.path(), input.as_bytes(), &mut output, false, None).unwrap();

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

        run_with_io(root.path(), input.as_bytes(), &mut output, false, None).unwrap();

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

        // Input: type non-existent path at workspace root prompt, then EOF for retry
        let input = format!("{}\n", nonexistent);
        let mut output = Vec::new();

        let result = run_with_io(&start, input.as_bytes(), &mut output, false, None);

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

        let result = run_with_io(root.path(), input.as_bytes(), &mut output, false, None);

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

    // ── MX-003 new tests ──────────────────────────────────────────────

    /// Step indicator: wizard prompts include [N/M] prefix.
    /// Happy path (has repos, no reinit) → [1/2] and [2/2].
    #[test]
    fn test_step_indicator_prefix() {
        let root = tempfile::tempdir().unwrap();
        make_git_repo(&root.path().join("repo-a"));
        make_git_repo(&root.path().join("repo-b"));

        let input = "\n\n"; // accept workspace (Enter), accept place (Enter)
        let mut output = Vec::new();

        run_with_io(root.path(), input.as_bytes(), &mut output, false, None).unwrap();

        let out = String::from_utf8(output).unwrap();
        assert!(
            out.contains("[1/2]"),
            "output must contain '[1/2]', got:\n{}",
            out
        );
        assert!(
            out.contains("[2/2]"),
            "output must contain '[2/2]', got:\n{}",
            out
        );
    }

    /// Error retry succeeds: first path does not exist, second path is valid.
    /// Verifies: wizard completes successfully and .mind-root created at second path.
    #[test]
    fn test_error_retry_succeeds() {
        let root = tempfile::tempdir().unwrap();
        make_git_repo(&root.path().join("repo-a"));

        // A second valid dir for the retry
        let second = tempfile::tempdir().unwrap();
        make_git_repo(&second.path().join("repo-x"));

        // Wizard starts at `root`, so detect_candidate(root) → root.
        // Input: type nonexistent path → retry with second path → accept place
        let input = format!("/nonexistent/path/9f3k2j1\n{}\n\n", second.path().display());
        let mut output = Vec::new();

        // start = root, but user types second.path() as the override
        run_with_io(root.path(), input.as_bytes(), &mut output, false, None).unwrap();

        // .mind-root must be at second path (where user retried to)
        assert!(
            second.path().join(".mind-root").exists(),
            ".mind-root must exist at retry path"
        );
        // NOT at root
        assert!(
            !root.path().join(".mind-root").exists(),
            ".mind-root must NOT exist at original root"
        );
    }

    /// Error retry fails: both paths do not exist → returns DirectoryNotFound (no infinite loop).
    #[test]
    fn test_error_retry_fails() {
        let root = tempfile::tempdir().unwrap();
        make_git_repo(&root.path().join("repo-a"));

        // Input: first nonexistent, retry also nonexistent
        let input = "/nonexistent/path/first\n/nonexistent/path/second\n";
        let mut output = Vec::new();

        let result = run_with_io(root.path(), input.as_bytes(), &mut output, false, None);

        assert!(
            matches!(result, Err(InitError::DirectoryNotFound(_))),
            "expected DirectoryNotFound after two failed attempts, got: {:?}",
            result
        );

        // Verify the output shows the error and retry prompt (not looped again)
        let out = String::from_utf8(output).unwrap();
        assert!(
            out.contains("Enter a different path:"),
            "output must contain retry prompt, got:\n{}",
            out
        );
    }

    /// --yes accepts detected root without prompting.
    /// Verifies: .mind-root created, no workspace prompt in output.
    #[test]
    fn test_yes_accepts_detected_root() {
        let root = tempfile::tempdir().unwrap();
        make_git_repo(&root.path().join("repo-a"));

        let mut output = Vec::new();
        // No input needed — --yes skips all prompts
        run_with_io(root.path(), "".as_bytes(), &mut output, true, None).unwrap();

        assert!(
            root.path().join(".mind-root").exists(),
            ".mind-root must exist after --yes init"
        );

        // No step indicator or workspace prompt in output
        let out = String::from_utf8(output).unwrap();
        assert!(
            !out.contains("[1/"),
            "output must not contain step indicators when --yes is set, got:\n{}",
            out
        );
        assert!(
            !out.contains("Workspace root ["),
            "output must not contain workspace prompt when --yes is set, got:\n{}",
            out
        );
    }

    /// --yes with no detectable candidate → actionable error.
    /// A tempdir with no git repos is not a valid workspace candidate.
    #[test]
    fn test_yes_no_candidate_errors() {
        // Tempdir with no git repo subdirs — detect_candidate falls back to start.
        // is_workspace_candidate(start) = !is_git_repo && count_git_repos > 0 = false.
        let root = tempfile::tempdir().unwrap();
        // Do NOT create any git repos under root.

        let mut output = Vec::new();
        let result = run_with_io(root.path(), "".as_bytes(), &mut output, true, None);

        assert!(
            matches!(result, Err(InitError::NoCandidate)),
            "expected NoCandidate, got: {:?}",
            result
        );

        // Error message must be actionable
        let msg = format!("{}", InitError::NoCandidate);
        assert!(
            msg.contains("--path"),
            "NoCandidate message must mention --path, got: {}",
            msg
        );
    }

    /// --path skips detection and uses the given path.
    /// Verifies: .mind-root created at explicit path, no "Detecting" output.
    #[test]
    fn test_path_skips_detection() {
        let explicit = tempfile::tempdir().unwrap();
        make_git_repo(&explicit.path().join("repo-a"));

        // start is a different dir (does not matter since --path overrides)
        let start = tempfile::tempdir().unwrap();

        // Input: accept place confirmation (Enter)
        let input = "\n";
        let mut output = Vec::new();

        run_with_io(
            start.path(),
            input.as_bytes(),
            &mut output,
            false,
            Some(explicit.path().to_str().unwrap()),
        )
        .unwrap();

        assert!(
            explicit.path().join(".mind-root").exists(),
            ".mind-root must exist at explicit path"
        );

        let out = String::from_utf8(output).unwrap();
        assert!(
            !out.contains("Detecting workspace root"),
            "output must not contain detection text when --path is set, got:\n{}",
            out
        );
    }

    /// --yes --path uses explicit path and emits no prompts.
    /// Verifies: .mind-root created, no step indicators or prompt text.
    #[test]
    fn test_yes_and_path_together() {
        let explicit = tempfile::tempdir().unwrap();
        make_git_repo(&explicit.path().join("repo-a"));

        let start = tempfile::tempdir().unwrap();

        let mut output = Vec::new();
        // No input needed — both --yes and --path skip all prompts
        run_with_io(
            start.path(),
            "".as_bytes(),
            &mut output,
            true,
            Some(explicit.path().to_str().unwrap()),
        )
        .unwrap();

        assert!(
            explicit.path().join(".mind-root").exists(),
            ".mind-root must exist after --yes --path init"
        );

        let out = String::from_utf8(output).unwrap();
        assert!(
            !out.contains("[1/"),
            "output must not contain step indicators when --yes --path is set, got:\n{}",
            out
        );
        assert!(
            !out.contains("Detecting workspace root"),
            "output must not contain detection text, got:\n{}",
            out
        );
    }
}
