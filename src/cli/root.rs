use crate::infra::workspace::{find_workspace_root, WorkspaceError};

// PHASE-2-BRIDGE Contract 1: mind root output format is frozen.
// Output: absolute path, no trailing slash, on stdout.
// Exit codes: 0 = found, 1 = not found, 2 = system error.
// DO NOT change this format without a new design session and version bump.
pub fn run() -> i32 {
    match find_workspace_root() {
        Ok(root) => {
            println!("{}", root.display());
            0
        }
        Err(WorkspaceError::NotFound) => {
            eprintln!("no workspace found. Run 'anchor init' to configure.");
            1
        }
        Err(e) => {
            eprintln!("error: {}", e);
            2
        }
    }
}
