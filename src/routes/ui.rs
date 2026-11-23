//! Serves the admin web UI (React build from `ui/dist`, embedded into the binary).

use axum::extract::Path;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "ui/dist"]
struct UiAssets;

pub async fn redirect_to_ui() -> Redirect {
    Redirect::temporary("/ui/")
}

pub async fn index() -> Response {
    serve("index.html")
}

pub async fn asset(Path(path): Path<String>) -> Response {
    if UiAssets::get(&path).is_some() {
        return serve(&path);
    }
    // Unknown non-asset paths fall back to the SPA shell.
    if path.contains('.') {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    serve("index.html")
}

fn serve(path: &str) -> Response {
    let Some(file) = UiAssets::get(path) else {
        return (
            StatusCode::NOT_FOUND,
            "FeatherTail UI was not built into this binary (run `npm run build` in ui/ and rebuild)",
        )
            .into_response();
    };
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    // Hashed assets are immutable; the shell must always be revalidated.
    let cache = if path.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    (
        [
            (header::CONTENT_TYPE, mime.as_ref().to_owned()),
            (header::CACHE_CONTROL, cache.to_owned()),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_owned()),
            (header::X_FRAME_OPTIONS, "DENY".to_owned()),
        ],
        file.data.into_owned(),
    )
        .into_response()
}
