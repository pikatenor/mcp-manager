use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, Request, StatusCode};
use axum::middleware::{from_fn_with_state, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::Router;
use mcp_core::TokenService;

use crate::auth::extract_bearer;

#[derive(Clone)]
pub struct AppState {
    pub tokens: Arc<Mutex<TokenService>>,
}

pub fn router(tokens: Arc<Mutex<TokenService>>) -> Router {
    let state = AppState { tokens };
    Router::new()
        .route("/mcp", post(mcp_placeholder))
        .route_layer(from_fn_with_state(state.clone(), require_token))
        .with_state(state)
}

pub async fn serve(tokens: Arc<Mutex<TokenService>>) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(mcp_core::DEFAULT_HTTP_BIND).await?;
    axum::serve(listener, router(tokens)).await
}

async fn require_token(State(state): State<AppState>, req: Request<Body>, next: Next) -> Response {
    let header = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    let Some(token) = extract_bearer(header) else {
        return (
            StatusCode::UNAUTHORIZED,
            axum::Json(serde_json::json!({ "error": "authentication required" })),
        )
            .into_response();
    };
    let valid = state
        .tokens
        .lock()
        .ok()
        .and_then(|svc| svc.validate(&token).ok())
        .is_some();
    if !valid {
        return (
            StatusCode::UNAUTHORIZED,
            axum::Json(serde_json::json!({ "error": "invalid token" })),
        )
            .into_response();
    }
    next.run(req).await
}

async fn mcp_placeholder() -> impl IntoResponse {
    (
        StatusCode::OK,
        axum::Json(serde_json::json!({ "ok": true })),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;
    use mcp_core::TokenService;
    use tower::ServiceExt;

    fn app_with_token() -> (Router, String) {
        let mut tokens = TokenService::new();
        let issued = tokens.issue("cursor");
        let plaintext = issued.plaintext.clone();
        (router(Arc::new(Mutex::new(tokens))), plaintext)
    }

    #[tokio::test]
    async fn missing_token_is_unauthorized() {
        let (app, _) = app_with_token();
        let res = app
            .oneshot(Request::post("/mcp").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn invalid_token_is_unauthorized() {
        let (app, _) = app_with_token();
        let res = app
            .oneshot(
                Request::post("/mcp")
                    .header(header::AUTHORIZATION, "Bearer mcpm_nope")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn revoked_token_is_unauthorized() {
        let mut tokens = TokenService::new();
        let issued = tokens.issue("cursor");
        tokens.revoke(&issued.id).unwrap();
        let app = router(Arc::new(Mutex::new(tokens)));
        let res = app
            .oneshot(
                Request::post("/mcp")
                    .header(
                        header::AUTHORIZATION,
                        format!("Bearer {}", issued.plaintext),
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn valid_token_reaches_handler() {
        let (app, token) = app_with_token();
        let res = app
            .oneshot(
                Request::post("/mcp")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = res.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["ok"], true);
    }
}
