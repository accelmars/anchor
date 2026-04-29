// src/refs.rs — shared reference pattern utilities

/// Returns all valid partial-path suffixes of `path` that anchor recognises as
/// partial-path matches during plain-text scanning.
///
/// A valid suffix starts after a `/` boundary in `src` (prevents `bar` from
/// matching `foobar`). The last path component (trailing segment) is always
/// included. Suffixes are returned longest-first.
///
/// Example: `"a/b/c"` → `["b/c", "c"]`
pub fn partial_path_segments(path: &str) -> Vec<String> {
    let parts: Vec<&str> = path.split('/').collect();
    let mut result = Vec::new();
    for n in 1..parts.len() {
        result.push(parts[n..].join("/"));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_component_returns_empty() {
        assert!(partial_path_segments("os-council").is_empty());
    }

    #[test]
    fn test_two_components() {
        assert_eq!(partial_path_segments("councils/os"), vec!["os"]);
    }

    #[test]
    fn test_three_components() {
        let segs = partial_path_segments("accelmars-guild/councils/os-council");
        assert_eq!(segs, vec!["councils/os-council", "os-council"]);
    }

    #[test]
    fn test_longest_first() {
        let segs = partial_path_segments("a/b/c/d");
        assert_eq!(segs, vec!["b/c/d", "c/d", "d"]);
    }
}
