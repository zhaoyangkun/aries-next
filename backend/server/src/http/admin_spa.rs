//! Admin SPA 静态资源服务：`apps/admin/dist` 构建产物通过 `include_dir` 编入 Binary，
//! 运行时从内存直出，无需文件系统挂载。挂载在 `/admin/` 下（Caddy 将 `/admin/*` 同源转发到本服务）。
//!
//! 本地未构建 Admin SPA 时 `build.rs` 会写入占位 `index.html`，保证 `cargo test/clippy` 可编译。

use axum::{
    extract::Path,
    http::{StatusCode, header},
    response::{IntoResponse, Redirect, Response},
};
use include_dir::{Dir, include_dir};

/// 构建产物根目录；相对 aries-server 的 CARGO_MANIFEST_DIR 解析。
static ADMIN_DIST: Dir = include_dir!("$CARGO_MANIFEST_DIR/../../apps/admin/dist");

const INDEX_HTML: &str = "index.html";
/// Vite 产物文件名带 Content Hash，可长缓存；index.html 必须每次校验，防发版后浏览器持旧入口。
const CACHE_IMMUTABLE: &str = "public, max-age=31536000, immutable";
const CACHE_NO_CACHE: &str = "no-cache";

/// `/admin` → `/admin/`：无尾斜杠相对资源路径会解析错，必须 308。
pub async fn redirect_to_admin() -> Redirect {
    Redirect::permanent("/admin/")
}

/// `/admin/` 单独注册：`{*path}` 通配不匹配空剩余路径，根路径必须有自己的路由。
pub async fn serve_root() -> Response {
    serve_index()
}

/// `/admin/{*path}`：命中真实文件直出；未命中且无扩展名视为 SPA 路由，回退 index.html；
/// 带扩展名未命中是真缺资源（如构建漂移），返回 404 而不是 HTML，避免把 JS 404 吞成文档。
pub async fn serve(Path(path): Path<String>) -> Response {
    let relative = path.trim_start_matches('/');
    if relative.is_empty() {
        return serve_index();
    }
    if let Some(file) = ADMIN_DIST.get_file(relative) {
        let cache = if relative == INDEX_HTML {
            CACHE_NO_CACHE
        } else {
            CACHE_IMMUTABLE
        };
        return file_response(file.contents(), content_type(relative), cache);
    }
    let has_extension = std::path::Path::new(relative)
        .extension()
        .is_some_and(|extension| !extension.is_empty());
    if has_extension {
        return StatusCode::NOT_FOUND.into_response();
    }
    serve_index()
}

fn serve_index() -> Response {
    let contents = ADMIN_DIST
        .get_file(INDEX_HTML)
        .map(|file| file.contents())
        .unwrap_or(b"");
    file_response(contents, "text/html; charset=utf-8", CACHE_NO_CACHE)
}

fn file_response(
    contents: &'static [u8],
    content_type: &'static str,
    cache_control: &'static str,
) -> Response {
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, cache_control),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        contents,
    )
        .into_response()
}

/// 常见静态资源扩展名 → Content-Type。未识别的二进制按 application/octet-stream 兜底。
fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("ttf") => "font/ttf",
        Some("txt") => "text/plain; charset=utf-8",
        Some("map") => "application/json; charset=utf-8",
        Some("webmanifest") => "application/manifest+json",
        Some("xml") => "application/xml",
        _ => "application/octet-stream",
    }
}
