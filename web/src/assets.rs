//! The SvelteKit SPA, embedded into the binary at build time.
//!
//! Release builds embed `../ui/build`; debug builds read it from disk (the
//! rust-embed default), so `npm run build` is picked up without recompiling.
//! If the frontend has not been built, the API still works and `/` explains
//! what is missing.

use axum::{
    body::Body,
    http::{HeaderMap, HeaderValue, StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use rust_embed::{EmbeddedFile, RustEmbed};

#[derive(RustEmbed)]
#[folder = "../ui/build"]
#[allow_missing = true]
struct Assets;

const INDEX: &str = "index.html";

pub async fn serve(uri: Uri, headers: HeaderMap) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { INDEX } else { path };

    if let Some(file) = Assets::get(path) {
        return file_response(path, &file, &headers);
    }
    // Client-side routes (`/settings`, `/firewall`, ...) get the SPA shell.
    // Anything that looks like a file name is a genuine 404.
    let last_segment = path.rsplit('/').next().unwrap_or(path);
    if last_segment.contains('.') {
        return StatusCode::NOT_FOUND.into_response();
    }
    match Assets::get(INDEX) {
        Some(index) => file_response(INDEX, &index, &headers),
        None => (
            StatusCode::SERVICE_UNAVAILABLE,
            "The web UI has not been built. Run `npm run build` in ui/ (or scripts/build-web.sh).\n",
        )
            .into_response(),
    }
}

fn file_response(path: &str, file: &EmbeddedFile, request_headers: &HeaderMap) -> Response {
    let etag = etag(&file.metadata.sha256_hash());
    let cache_control = if path.starts_with("_app/immutable/") {
        "public, max-age=31536000, immutable"
    } else {
        // index.html and friends must be revalidated so upgrades show up.
        "no-cache"
    };

    let not_modified = request_headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').any(|t| t.trim() == etag));

    let mut response = if not_modified {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        Response::new(Body::from(file.data.clone()))
    };
    let headers = response.headers_mut();
    if let Ok(v) = HeaderValue::from_str(&etag) {
        headers.insert(header::ETAG, v);
    }
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(cache_control),
    );
    if !not_modified && let Ok(v) = HeaderValue::from_str(file.metadata.mimetype()) {
        headers.insert(header::CONTENT_TYPE, v);
    }
    response
}

fn etag(hash: &[u8; 32]) -> String {
    let hex: String = hash[..16].iter().map(|b| format!("{b:02x}")).collect();
    format!("\"{hex}\"")
}
