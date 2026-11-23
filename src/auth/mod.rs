use std::collections::HashMap;
use std::sync::{Mutex, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::routes::AppState;

pub const SESSION_COOKIE: &str = "ft_session";
const SESSION_TTL_SECS: i64 = 12 * 60 * 60;
const MIN_PASSWORD_LEN: usize = 8;

/// Auth state for both the FeatherPanel bearer token and the admin web UI sessions.
pub struct AdminAuth {
    api_token: String,
    config_path: String,
    password_hash: RwLock<Option<String>>,
    sessions: Mutex<HashMap<String, i64>>,
}

impl AdminAuth {
    pub fn new(api_token: String, password_hash: Option<String>, config_path: String) -> Self {
        Self {
            api_token,
            config_path,
            password_hash: RwLock::new(password_hash.filter(|h| !h.trim().is_empty())),
            sessions: Mutex::new(HashMap::new()),
        }
    }

    pub fn token_matches(&self, candidate: &str) -> bool {
        let token = self.api_token.trim();
        // The placeholder token must never unlock anything.
        if token.is_empty() || token == "change-me" {
            return false;
        }
        constant_time_eq(candidate.as_bytes(), token.as_bytes())
    }

    pub fn has_password(&self) -> bool {
        self.password_hash
            .read()
            .map(|h| h.is_some())
            .unwrap_or(false)
    }

    /// Accepts either the admin password or the daemon api_token.
    pub fn verify_login(&self, password: &str) -> Option<&'static str> {
        if self.token_matches(password) {
            return Some("token");
        }
        let hash = self.password_hash.read().ok()?.clone()?;
        match bcrypt::verify(password, &hash) {
            Ok(true) => Some("password"),
            _ => None,
        }
    }

    pub fn create_session(&self) -> Result<String, String> {
        let mut raw = [0u8; 32];
        getrandom::getrandom(&mut raw).map_err(|e| e.to_string())?;
        let id: String = raw.iter().map(|b| format!("{b:02x}")).collect();
        let now = now_unix();
        let mut sessions = self.sessions.lock().map_err(|e| e.to_string())?;
        sessions.retain(|_, exp| *exp > now);
        sessions.insert(id.clone(), now + SESSION_TTL_SECS);
        Ok(id)
    }

    pub fn session_valid(&self, id: &str) -> bool {
        let now = now_unix();
        let Ok(mut sessions) = self.sessions.lock() else {
            return false;
        };
        match sessions.get(id) {
            Some(exp) if *exp > now => true,
            Some(_) => {
                sessions.remove(id);
                false
            }
            None => false,
        }
    }

    pub fn destroy_session(&self, id: &str) {
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.remove(id);
        }
    }

    pub fn destroy_all_sessions(&self) {
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.clear();
        }
    }

    /// Hashes and persists a new admin password into the config file (comments preserved).
    pub fn set_password(&self, password: &str) -> Result<(), String> {
        let hash = hash_password(password)?;
        write_password_hash(&self.config_path, &hash)?;
        if let Ok(mut guard) = self.password_hash.write() {
            *guard = Some(hash);
        }
        Ok(())
    }
}

pub fn hash_password(password: &str) -> Result<String, String> {
    if password.chars().count() < MIN_PASSWORD_LEN {
        return Err(format!("password must be at least {MIN_PASSWORD_LEN} characters"));
    }
    bcrypt::hash(password, 11).map_err(|e| e.to_string())
}

/// Writes `auth.admin_password_hash` into the TOML config, keeping comments and layout.
pub fn write_password_hash(config_path: &str, hash: &str) -> Result<(), String> {
    let contents = std::fs::read_to_string(config_path).unwrap_or_default();
    let mut doc: toml_edit::DocumentMut = contents
        .parse()
        .map_err(|e| format!("failed to parse {config_path}: {e}"))?;
    if !doc.contains_table("auth") {
        doc["auth"] = toml_edit::table();
    }
    doc["auth"]["admin_password_hash"] = toml_edit::value(hash);
    std::fs::write(config_path, doc.to_string())
        .map_err(|e| format!("failed to write {config_path}: {e}"))
}

/// Accepts `Authorization: Bearer <api_token>` (FeatherPanel / scripts) or a web UI session cookie.
pub async fn require_auth(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: Request,
    next: Next,
) -> Result<Response, (StatusCode, Json<serde_json::Value>)> {
    if let Some(token) = extract_bearer(&headers) {
        if state.auth.token_matches(&token) {
            return Ok(next.run(request).await);
        }
        return Err(unauthorized_response());
    }

    if let Some(session) = extract_cookie(&headers, SESSION_COOKIE) {
        if state.auth.session_valid(&session) {
            return Ok(next.run(request).await);
        }
    }

    Err(unauthorized_response())
}

#[derive(Deserialize)]
pub struct LoginRequest {
    pub password: String,
}

#[derive(Serialize)]
pub struct SessionInfo {
    pub authenticated: bool,
    pub password_configured: bool,
    pub method: Option<String>,
}

pub async fn login(State(state): State<AppState>, Json(payload): Json<LoginRequest>) -> Response {
    let Some(method) = state.auth.verify_login(&payload.password) else {
        warn!("admin ui login failed");
        // Slow down brute force attempts.
        tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
        return unauthorized_response().into_response();
    };

    let session = match state.auth.create_session() {
        Ok(s) => s,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": err })),
            )
                .into_response();
        }
    };

    info!(method, "admin ui login succeeded");
    let cookie = format!(
        "{SESSION_COOKIE}={session}; Path=/; HttpOnly; SameSite=Strict; Max-Age={SESSION_TTL_SECS}"
    );
    let mut response = Json(SessionInfo {
        authenticated: true,
        password_configured: state.auth.has_password(),
        method: Some(method.to_owned()),
    })
    .into_response();
    if let Ok(value) = HeaderValue::from_str(&cookie) {
        response.headers_mut().insert(header::SET_COOKIE, value);
    }
    response
}

pub async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(session) = extract_cookie(&headers, SESSION_COOKIE) {
        state.auth.destroy_session(&session);
    }
    let mut response = Json(serde_json::json!({ "ok": true })).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_static("ft_session=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0"),
    );
    response
}

pub async fn session(State(state): State<AppState>, headers: HeaderMap) -> Json<SessionInfo> {
    let authenticated = extract_cookie(&headers, SESSION_COOKIE)
        .map(|s| state.auth.session_valid(&s))
        .unwrap_or(false);
    Json(SessionInfo {
        authenticated,
        password_configured: state.auth.has_password(),
        method: None,
    })
}

#[derive(Deserialize)]
pub struct ChangePasswordRequest {
    pub new_password: String,
}

/// Requires an authenticated session/token (mounted behind `require_auth`).
pub async fn change_password(
    State(state): State<AppState>,
    Json(payload): Json<ChangePasswordRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    state
        .auth
        .set_password(&payload.new_password)
        .map_err(|err| (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": err }))))?;
    // Force everyone (including this browser) to log in again with the new password.
    state.auth.destroy_all_sessions();
    info!("admin ui password changed");
    Ok(Json(serde_json::json!({ "ok": true })))
}

fn unauthorized_response() -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({ "error": "unauthorized" })),
    )
}

fn extract_bearer(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(header::AUTHORIZATION)?;
    let value = raw.to_str().ok()?;
    let token = value.strip_prefix("Bearer ")?;
    Some(token.to_owned())
}

fn extract_cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    for raw in headers.get_all(header::COOKIE) {
        let Ok(value) = raw.to_str() else { continue };
        for part in value.split(';') {
            let mut kv = part.trim().splitn(2, '=');
            if kv.next() == Some(name) {
                if let Some(v) = kv.next() {
                    if !v.is_empty() {
                        return Some(v.to_owned());
                    }
                }
            }
        }
    }
    None
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
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

    #[test]
    fn extract_cookie_finds_session() {
        let mut headers = HeaderMap::new();
        headers.insert(header::COOKIE, HeaderValue::from_static("a=1; ft_session=xyz; b=2"));
        assert_eq!(extract_cookie(&headers, SESSION_COOKIE).as_deref(), Some("xyz"));
    }

    #[test]
    fn placeholder_token_never_matches() {
        let auth = AdminAuth::new("change-me".into(), None, String::new());
        assert!(!auth.token_matches("change-me"));
        assert!(auth.verify_login("change-me").is_none());
    }

    #[test]
    fn sessions_roundtrip() {
        let auth = AdminAuth::new("tok-123456".into(), None, String::new());
        assert_eq!(auth.verify_login("tok-123456"), Some("token"));
        let s = auth.create_session().unwrap();
        assert!(auth.session_valid(&s));
        auth.destroy_session(&s);
        assert!(!auth.session_valid(&s));
    }

    #[test]
    fn password_hash_written_preserving_comments() {
        let path = std::env::temp_dir().join(format!("ft-auth-{}.toml", now_unix()));
        std::fs::write(&path, "# keep me\n[auth]\napi_token = \"x\"\n").unwrap();
        write_password_hash(path.to_str().unwrap(), "$2b$hash").unwrap();
        let out = std::fs::read_to_string(&path).unwrap();
        assert!(out.contains("# keep me"));
        assert!(out.contains("admin_password_hash = \"$2b$hash\""));
        let _ = std::fs::remove_file(&path);
    }
}
