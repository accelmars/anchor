/// Compute Levenshtein edit distance between two byte sequences.
fn levenshtein(a: &str, b: &str) -> usize {
    let a = a.as_bytes();
    let b = b.as_bytes();
    let m = a.len();
    let n = b.len();

    if m == 0 {
        return n;
    }
    if n == 0 {
        return m;
    }

    let mut prev: Vec<usize> = (0..=n).collect();
    let mut curr = vec![0usize; n + 1];

    for i in 1..=m {
        curr[0] = i;
        for j in 1..=n {
            curr[j] = if a[i - 1] == b[j - 1] {
                prev[j - 1]
            } else {
                1 + prev[j - 1].min(prev[j]).min(curr[j - 1])
            };
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    prev[n]
}

/// Extract the final path component (basename) from a path string.
/// Strips trailing slashes before extracting.
fn basename(path: &str) -> &str {
    let path = path.trim_end_matches('/');
    match path.rfind('/') {
        Some(pos) => &path[pos + 1..],
        None => path,
    }
}

/// Return the number of leading path components shared between two paths.
fn prefix_overlap(a: &str, b: &str) -> usize {
    a.split('/')
        .zip(b.split('/'))
        .take_while(|(x, y)| x == y)
        .count()
}

/// Return up to 3 workspace-relative candidate paths closest to `missing`.
/// Candidates with normalized Levenshtein distance > 0.6 on basename are excluded.
/// Ties broken by path prefix overlap (higher = better).
pub fn suggest_similar(missing: &str, candidates: &[String]) -> Vec<String> {
    let missing_base = basename(missing);

    let mut scored: Vec<(f64, usize, &String)> = candidates
        .iter()
        .filter_map(|candidate| {
            let candidate_base = basename(candidate.as_str());
            let max_len = missing_base.len().max(candidate_base.len());
            if max_len == 0 {
                // Both basenames empty — treat as exact match.
                return Some((0.0, prefix_overlap(missing, candidate.as_str()), candidate));
            }
            let dist = levenshtein(missing_base, candidate_base);
            let normalized = dist as f64 / max_len as f64;
            if normalized > 0.6 {
                return None;
            }
            let overlap = prefix_overlap(missing, candidate.as_str());
            Some((normalized, overlap, candidate))
        })
        .collect();

    // Sort: normalized distance ascending (lower = better),
    // then prefix overlap descending (higher = better).
    scored.sort_by(|a, b| {
        a.0.partial_cmp(&b.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.1.cmp(&a.1))
    });

    scored
        .into_iter()
        .take(3)
        .map(|(_, _, c)| c.clone())
        .collect()
}

/// Format a "Did you mean?" stderr block.
///
/// Output (when suggestions non-empty):
///   Could not find '{missing}'
///
///   Did you mean?
///     1. {suggestion1}
///     2. {suggestion2}   (if present)
///     3. {suggestion3}   (if present)
///
///   {corrected_command}   (if Some)
///
/// Output (when suggestions empty):
///   Could not find '{missing}'
pub fn format_suggestions(
    missing: &str,
    suggestions: &[String],
    corrected_command: Option<&str>,
) -> String {
    if suggestions.is_empty() {
        return format!("Could not find '{missing}'");
    }

    let mut output = format!("Could not find '{missing}'\n\nDid you mean?\n");
    for (i, s) in suggestions.iter().enumerate() {
        output.push_str(&format!("  {}. {}\n", i + 1, s));
    }
    if let Some(cmd) = corrected_command {
        output.push_str(&format!("\n{cmd}\n"));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- suggest_similar tests ---

    #[test]
    fn test_exact_basename_match_always_included() {
        let result = suggest_similar("docs/design.md", &["docs/design.md".to_string()]);
        assert_eq!(result, vec!["docs/design.md"]);
    }

    #[test]
    fn test_single_char_typo_included() {
        // Exit criterion 1: "anchor-foundtion" vs ["anchor-foundation/", "other.md"]
        // basename("anchor-foundation/") = "anchor-foundation"
        // levenshtein("anchor-foundtion", "anchor-foundation") = 1 → normalized 1/17 ≈ 0.06 ≤ 0.6
        // basename("other.md") = "other.md"
        // levenshtein("anchor-foundtion", "other.md") >> 0.6 → excluded
        let result = suggest_similar(
            "anchor-foundtion",
            &["anchor-foundation/".to_string(), "other.md".to_string()],
        );
        assert_eq!(result, vec!["anchor-foundation/"]);
    }

    #[test]
    fn test_unrelated_name_excluded() {
        // Exit criterion 2: "xyz123qwe" vs ["anchor-foundation/"] → []
        let result = suggest_similar("xyz123qwe", &["anchor-foundation/".to_string()]);
        assert!(result.is_empty());
    }

    #[test]
    fn test_top_3_of_5_close_candidates_returned() {
        // Exit criterion 3: 5 close candidates → exactly 3 returned
        // "desig.md" vs "design.md":  dist=1, max=9, normalized=0.11 ≤ 0.6
        // "desig.md" vs "design1.md": dist=2, max=10, normalized=0.20 ≤ 0.6
        // (same for design2..4)
        let candidates: Vec<String> = vec![
            "design.md".to_string(),
            "design1.md".to_string(),
            "design2.md".to_string(),
            "design3.md".to_string(),
            "design4.md".to_string(),
        ];
        let result = suggest_similar("desig.md", &candidates);
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn test_empty_candidates_returns_empty() {
        let result = suggest_similar("anything.md", &[]);
        assert!(result.is_empty());
    }

    // --- format_suggestions tests ---

    #[test]
    fn test_format_suggestions_with_corrected_command() {
        // Exit criterion 4: output contains "Did you mean?" and numbered list
        let suggestions = vec![
            "anchor-foundation/".to_string(),
            "anchor-forge/".to_string(),
        ];
        let output = format_suggestions(
            "anchor-foundtion",
            &suggestions,
            Some("Try: anchor file mv anchor-foundation/ ..."),
        );
        assert!(output.contains("Could not find 'anchor-foundtion'"));
        assert!(output.contains("Did you mean?"));
        assert!(output.contains("1. anchor-foundation/"));
        assert!(output.contains("2. anchor-forge/"));
        assert!(output.contains("Try: anchor file mv anchor-foundation/ ..."));
    }

    #[test]
    fn test_format_suggestions_empty_no_did_you_mean() {
        // Exit criterion 5: empty → "Could not find" line only, no "Did you mean?" block
        let output = format_suggestions("xyz123qwe", &[], None);
        assert_eq!(output, "Could not find 'xyz123qwe'");
        assert!(!output.contains("Did you mean?"));
    }
}
