use axum::{
    Json,
    http::{StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use rust_embed::Embed;
use serde_json::json;

#[derive(Embed)]
#[folder = "../frontend/build"]
struct Assets;

pub async fn serve(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if path.starts_with("api/") {
        return (StatusCode::NOT_FOUND, Json(json!({ "error": { "code": "not_found", "message": "No such endpoint" } })))
            .into_response();
    }
    let (file, path) = match Assets::get(path).filter(|_| !path.is_empty()) {
        Some(f) => (f, path),
        None => match Assets::get("200.html") {
            Some(f) => (f, "200.html"),
            None => return (StatusCode::NOT_FOUND, "Frontend not built; run `npm run build` in frontend/").into_response(),
        },
    };
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let cache = if path.starts_with("_app/immutable/") { "public, max-age=31536000, immutable" } else { "no-cache" };
    ([(header::CONTENT_TYPE, mime.as_ref()), (header::CACHE_CONTROL, cache)], file.data).into_response()
}
