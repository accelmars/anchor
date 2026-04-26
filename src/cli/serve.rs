use crate::infra::workspace;
use crate::server::{build_router, AnchorState};
use std::sync::Arc;

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

    let state = AnchorState {
        workspace_root: Arc::new(workspace_root),
    };
    let app = build_router(state);

    let listener = match tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("error: bind failed on port {port}: {e}");
            return 1;
        }
    };

    println!("Anchor serving on http://0.0.0.0:{port}");

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
