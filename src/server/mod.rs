use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Shared state for the anchor HTTP server.
#[derive(Clone)]
pub struct AnchorState {
    pub workspace_root: Arc<PathBuf>,
}

/// Build the axum router with all anchor HTTP endpoints.
pub fn build_router(state: AnchorState) -> Router {
    Router::new()
        .route("/health", get(handle_health))
        .route("/file/validate", post(handle_file_validate))
        .with_state(state)
}

/// Platform composition interface: returns a configured axum Router for this engine.
pub fn routes(state: AnchorState) -> Router {
    build_router(state)
}

/// Platform composition interface: build AnchorState from a workspace root path.
pub fn build_state(workspace_root: &Path) -> AnchorState {
    AnchorState {
        workspace_root: Arc::new(workspace_root.to_path_buf()),
    }
}

async fn handle_health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

#[derive(Serialize)]
struct BrokenRef {
    file: String,
    line: usize,
    #[serde(rename = "ref")]
    ref_target: String,
}

#[derive(Serialize)]
struct ValidateResponse {
    broken_refs: Vec<BrokenRef>,
}

async fn handle_file_validate(State(state): State<AnchorState>) -> impl IntoResponse {
    let root = state.workspace_root.as_ref().clone();
    let result = tokio::task::spawn_blocking(move || {
        crate::cli::file::validate::validate_workspace(&root)
    })
    .await;

    match result {
        Ok(Ok(broken)) => {
            let refs: Vec<BrokenRef> = broken
                .into_iter()
                .map(|(file, line, ref_target)| BrokenRef {
                    file,
                    line,
                    ref_target,
                })
                .collect();
            (StatusCode::OK, Json(ValidateResponse { broken_refs: refs })).into_response()
        }
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e})),
        )
            .into_response(),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "internal error"})),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use std::sync::Arc;
    use tempfile::TempDir;
    use tower::ServiceExt;

    #[test]
    fn test_build_state_from_path() {
        let tmp = TempDir::new().unwrap();
        let state = build_state(tmp.path());
        assert_eq!(*state.workspace_root, tmp.path().to_path_buf());
    }

    #[tokio::test]
    async fn test_routes_returns_router() {
        let tmp = TempDir::new().unwrap();
        let router: Router = routes(build_state(tmp.path()));
        let response = router
            .oneshot(Request::get("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_health_endpoint() {
        let tmp = TempDir::new().unwrap();
        let state = AnchorState {
            workspace_root: Arc::new(tmp.path().to_path_buf()),
        };
        let app = build_router(state);
        let response = app
            .oneshot(Request::get("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["status"], "ok");
        assert!(json["version"].is_string());
    }

    #[tokio::test]
    async fn test_validate_endpoint_empty_workspace() {
        let tmp = TempDir::new().unwrap();
        let state = AnchorState {
            workspace_root: Arc::new(tmp.path().to_path_buf()),
        };
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::post("/file/validate")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(json["broken_refs"].is_array());
        assert_eq!(json["broken_refs"].as_array().unwrap().len(), 0);
    }
}
