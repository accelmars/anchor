// src/core/fence_state.rs — markdown fenced code block state machine
//
// Tracks whether the parser is currently inside a fenced code block.
// Called by parser::parse_references on each line before ref matching.
//
// Rules (CommonMark §4.5/§4.6):
//   Opening fence: ≥3 identical fence chars (` or ~), optional leading spaces,
//                  optional info string after chars.
//   Closing fence: same character as opener, length ≥ opener length,
//                  only whitespace after the run.
//   Mismatched character: a tilde line does NOT close a backtick fence, and vice versa.

/// Which character opened the current fence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FenceMarker {
    Backtick,
    Tilde,
}

/// Per-file fence tracking state. Reset to `default()` at the start of each file.
#[derive(Debug, Default)]
pub(crate) struct FenceState {
    in_block: bool,
    marker: Option<FenceMarker>,
    marker_len: usize,
    /// Leading-space indent of the opening fence line (for close-fence indent check).
    marker_indent: usize,
}

impl FenceState {
    /// Update state by observing `line`.
    ///
    /// Strips a trailing `\r` before processing, so CRLF and LF files behave identically.
    /// Must be called for EVERY line — including blank lines and delimiter lines themselves —
    /// before calling `in_code_block()` for that line.
    pub(crate) fn observe_line(&mut self, line: &str) {
        let line = line.trim_end_matches('\r');

        let indent = line.bytes().take_while(|&b| b == b' ').count();
        let trimmed = &line[indent..];

        if self.in_block {
            let Some(ref marker) = self.marker.clone() else {
                return;
            };
            let fence_char = match marker {
                FenceMarker::Backtick => b'`',
                FenceMarker::Tilde => b'~',
            };
            let run_len = trimmed.bytes().take_while(|&b| b == fence_char).count();
            // Closing fence: same char, length ≥ opener, trailing whitespace only
            if run_len >= self.marker_len {
                let after = &trimmed[run_len..];
                if after.bytes().all(|b| b == b' ' || b == b'\t') {
                    self.in_block = false;
                    self.marker = None;
                    self.marker_len = 0;
                    self.marker_indent = 0;
                }
            }
        } else {
            let (fence_char, fence_marker) = if trimmed.starts_with('`') {
                (b'`', FenceMarker::Backtick)
            } else if trimmed.starts_with('~') {
                (b'~', FenceMarker::Tilde)
            } else {
                return;
            };

            let run_len = trimmed.bytes().take_while(|&b| b == fence_char).count();
            if run_len < 3 {
                return;
            }

            // Backtick opener: info string must not contain a backtick (CommonMark §4.5)
            if fence_char == b'`' {
                let after = &trimmed[run_len..];
                if after.contains('`') {
                    return;
                }
            }

            self.in_block = true;
            self.marker = Some(fence_marker);
            self.marker_len = run_len;
            self.marker_indent = indent;
        }
    }

    /// Returns `true` when the parser is currently inside a fenced code block.
    pub(crate) fn in_code_block(&self) -> bool {
        self.in_block
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fs() -> FenceState {
        FenceState::default()
    }

    // ── basic open/close round-trips ────────────────────────────────────────

    #[test]
    fn test_backtick_open_close() {
        let mut s = fs();
        s.observe_line("```");
        assert!(s.in_code_block());
        s.observe_line("content line");
        assert!(s.in_code_block());
        s.observe_line("```");
        assert!(!s.in_code_block());
    }

    #[test]
    fn test_tilde_open_close() {
        let mut s = fs();
        s.observe_line("~~~");
        assert!(s.in_code_block());
        s.observe_line("~~~");
        assert!(!s.in_code_block());
    }

    #[test]
    fn test_info_string_on_open() {
        let mut s = fs();
        s.observe_line("```bash");
        assert!(s.in_code_block());
        s.observe_line("```");
        assert!(!s.in_code_block());
    }

    #[test]
    fn test_tilde_with_info_string() {
        let mut s = fs();
        s.observe_line("~~~rust");
        assert!(s.in_code_block());
        s.observe_line("~~~");
        assert!(!s.in_code_block());
    }

    // ── mismatched markers ──────────────────────────────────────────────────

    #[test]
    fn test_mismatched_markers_tilde_does_not_close_backtick() {
        let mut s = fs();
        s.observe_line("```");
        assert!(s.in_code_block());
        s.observe_line("~~~"); // tilde must NOT close a backtick fence
        assert!(s.in_code_block(), "tilde line must not close a backtick fence");
        s.observe_line("```"); // correct closer
        assert!(!s.in_code_block());
    }

    #[test]
    fn test_mismatched_markers_backtick_does_not_close_tilde() {
        let mut s = fs();
        s.observe_line("~~~");
        assert!(s.in_code_block());
        s.observe_line("```"); // backtick must NOT close a tilde fence
        assert!(s.in_code_block(), "backtick line must not close a tilde fence");
        s.observe_line("~~~");
        assert!(!s.in_code_block());
    }

    // ── length variations ───────────────────────────────────────────────────

    #[test]
    fn test_longer_close_closes_fence() {
        let mut s = fs();
        s.observe_line("```");   // 3-backtick opener
        assert!(s.in_code_block());
        s.observe_line("````");  // 4-backtick line: length ≥ 3 → closes
        assert!(!s.in_code_block(), "4-backtick line must close 3-backtick opener");
    }

    #[test]
    fn test_shorter_close_does_not_close_fence() {
        let mut s = fs();
        s.observe_line("````");  // 4-backtick opener
        assert!(s.in_code_block());
        s.observe_line("```");   // 3-backtick line: length < 4 → does NOT close
        assert!(s.in_code_block(), "3-backtick line must not close 4-backtick opener");
        s.observe_line("````");  // correct closer
        assert!(!s.in_code_block());
    }

    // ── indented fences ─────────────────────────────────────────────────────

    #[test]
    fn test_indented_fence_inside_list() {
        let mut s = fs();
        s.observe_line("    ```bash"); // 4-space indent (list item body)
        assert!(s.in_code_block());
        s.observe_line("    code here");
        assert!(s.in_code_block());
        s.observe_line("    ```"); // same indent → closes
        assert!(!s.in_code_block());
    }

    #[test]
    fn test_two_space_indent_fence() {
        let mut s = fs();
        s.observe_line("  ```");
        assert!(s.in_code_block());
        s.observe_line("  ```");
        assert!(!s.in_code_block());
    }

    // ── CRLF handling ───────────────────────────────────────────────────────

    #[test]
    fn test_crlf_open_close() {
        let mut s = fs();
        s.observe_line("```\r");
        assert!(s.in_code_block());
        s.observe_line("```\r");
        assert!(!s.in_code_block());
    }

    #[test]
    fn test_crlf_with_info_string() {
        let mut s = fs();
        s.observe_line("```bash\r");
        assert!(s.in_code_block());
        s.observe_line("```\r");
        assert!(!s.in_code_block());
    }

    // ── edge cases ──────────────────────────────────────────────────────────

    #[test]
    fn test_two_backticks_not_a_fence() {
        let mut s = fs();
        s.observe_line("``");
        assert!(!s.in_code_block(), "two backticks are not a fence opener");
    }

    #[test]
    fn test_sequential_fences() {
        let mut s = fs();
        // First block
        s.observe_line("```");
        assert!(s.in_code_block());
        s.observe_line("```");
        assert!(!s.in_code_block());
        // Second block
        s.observe_line("~~~");
        assert!(s.in_code_block());
        s.observe_line("~~~");
        assert!(!s.in_code_block());
    }

    #[test]
    fn test_backtick_info_string_with_backtick_is_not_fence() {
        // Info string containing backtick → not a valid opening fence
        let mut s = fs();
        s.observe_line("``` `code`"); // info string contains backtick → invalid opener
        assert!(!s.in_code_block());
    }
}
