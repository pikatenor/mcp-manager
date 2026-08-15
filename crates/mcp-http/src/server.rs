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
    unimplemented!("router")
}

async fn require_token(
    State(state): State<AppState>,
    mut req: Request<Body>,
    next: Next,
) -> Response {
    unimplemented!("require_token")
}

async fn mcp_placeholder() -> impl IntoResponse {
    (StatusCode::OK, axum::Json(serde_json::json!({ "ok": true })))
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
        (
            router(Arc::new(Mutex::new(tokens))),
            plaintext,
        )
    }

    #[tokio::test]
    async fn missing_token_is_unauthorized() {
        let (app, _) = app_with_token();
        let res = app
            .oneshot(
                Request::post("/mcp")
                    .body(Body::empty())
                    .unwrap(),
            )
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
                    .header(header::AUTHORIZATION, format!("Bearer {}", issued.plaintext))
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
