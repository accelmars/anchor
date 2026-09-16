use crate::infra::workspace;
use crate::server::{build_router, AnchorState};
use std::sync::Arc;

/// Default bind host for `anchor serve`: **loopback only**.
///
/// The API is unauthenticated. It is read-only today (`/health`, `/file/validate`), so exposure
/// leaks workspace file paths rather than contents, and lets an unauthenticated caller trigger
/// repeated whole-workspace scans. That is tolerable on loopback and is not a decision to make on
/// a user's behalf on a shared network, so exposing it is opt-in — `--host 0.0.0.0`.
///
/// If a write endpoint is ever added here, this server needs authentication before it ships, not
/// a louder warning.
pub const DEFAULT_BIND_HOST: &str = "127.0.0.1";

pub fn run(host: &str, port: u16) -> i32 {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("error: failed to build runtime: {e}");
            return 1;
        }
    };
    rt.block_on(serve_async(host, port))
}

async fn serve_async(host: &str, port: u16) -> i32 {
    let workspace_root = match workspace::find_workspace_root() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let cwd = std::env::current_dir().unwrap_or_else(|_| workspace_root.clone());
    let engine_home = workspace::resolve(&cwd, workspace::ResolveHints { tenant_flag: None })
        .map(|r| r.engine_home)
        .unwrap_or_else(|_| workspace_root.join(".accelmars"));

    let state = AnchorState {
        workspace_root: Arc::new(workspace_root),
        engine_home: Arc::new(engine_home),
    };
    let app = build_router(state);

    let bind_addr = format!("{host}:{port}");
    let listener = match tokio::net::TcpListener::bind(&bind_addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("error: bind failed on {bind_addr}: {e}");
            return 1;
        }
    };

    // Report the address actually bound, not the one requested: with `--port 0` the OS picks an
    // ephemeral port, and the requested value is then not the one a caller can connect to.
    let bound = listener
        .local_addr()
        .map(|addr| addr.to_string())
        .unwrap_or_else(|_| bind_addr.clone());
    println!("Anchor serving on {bound}");
    if host != DEFAULT_BIND_HOST {
        println!(
            "warning: bound to {host}, not loopback. Anchor's API is unauthenticated: anyone who \
             can reach this address can read the paths of workspace files that contain broken \
             references, and can make this process rescan the workspace repeatedly."
        );
    }

    match axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
    {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: server error: {e}");
            1
        }
    }
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to install Ctrl-C handler");
}
