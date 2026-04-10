use axum::extract::Request;
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::Json;
use axum::response::Response;

use crate::routes::AppState;

pub async fn require_bearer_auth(
    axum::extract::State(state): axum::extract::State<AppState>,
    headers: HeaderMap,
    request: Request,
    next: Next,
) -> Result<Response, (StatusCode, Json<serde_json::Value>)> {
    let token = extract_bearer(&headers).ok_or_else(unauthorized_response)?;

    if token != state.auth_token {
        return Err(unauthorized_response());
    }

    Ok(next.run(request).await)
}

fn unauthorized_response() -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({ "error": "unauthorized" })),
    )
}

fn extract_bearer(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(axum::http::header::AUTHORIZATION)?;
    let value = raw.to_str().ok()?;
    let token = value.strip_prefix("Bearer ")?;
    Some(token.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderValue, header::AUTHORIZATION};

    #[test]
    fn extract_bearer_parses_valid_header() {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer abc123"));

        let token = extract_bearer(&headers);
        assert_eq!(token.as_deref(), Some("abc123"));
    }

    #[test]
    fn extract_bearer_rejects_invalid_prefix() {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Token abc123"));

        assert!(extract_bearer(&headers).is_none());
    }

    #[test]
    fn unauthorized_response_shape_is_stable() {
        let (status, body) = unauthorized_response();
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body.0["error"], "unauthorized");
    }
}
