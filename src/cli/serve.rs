use crate::infra::workspace;
use crate::server::{build_router, AnchorState};
use std::sync::Arc;

/// Wildcard bind host for `anchor serve`. Stated once so the bind call and the startup banner
/// cannot drift apart.
const BIND_HOST: &str = "0.0.0.0";

pub fn run(port: u16) -> i32 {
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
    rt.block_on(serve_async(port))
}

async fn serve_async(port: u16) -> i32 {
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

    let bind_addr = format!("{BIND_HOST}:{port}");
    let listener = match tokio::net::TcpListener::bind(&bind_addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("error: bind failed on {bind_addr}: {e}");
            return 1;
        }
    };

    // Report the address actually bound, not the one requested: with `--port 0` the OS picks an
    // ephemeral port, and the requested value is then not the one a caller can connect to.
    match listener.local_addr() {
        Ok(addr) => println!("Anchor serving on {addr}"),
        Err(_) => println!("Anchor serving on {bind_addr}"),
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
