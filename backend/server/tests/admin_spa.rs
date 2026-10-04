//! Admin SPA 静态服务 Contract Test：/admin 重定向、index.html 直出、SPA 路由回退、
//! 缺失静态资源 404。测试不依赖真实构建产物——本地未构建时 build.rs 写入占位 index.html。

mod common;

use anyhow::ensure;
use axum::http::{HeaderMap, StatusCode, header};

/// 从 HeaderMap 取字符串头，断言存在并返回。
fn header_str(headers: &HeaderMap, name: header::HeaderName) -> anyhow::Result<&str> {
    Ok(headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or(""))
}

#[tokio::test]
async fn admin_spa_serving_contract() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        // /admin → 308 到 /admin/（无尾斜杠相对资源路径会解析错）。
        let (status, headers, _) = app.get_raw("/admin").await?;
        ensure!(
            status == StatusCode::PERMANENT_REDIRECT,
            "expected 308, got {status}"
        );
        ensure!(
            header_str(&headers, header::LOCATION)? == "/admin/",
            "unexpected location: {:?}",
            headers.get(header::LOCATION)
        );

        // /admin/ → 200 text/html，且禁止缓存（发版后浏览器必须重新校验入口）。
        let (status, headers, body) = app.get_raw("/admin/").await?;
        ensure!(status == StatusCode::OK, "got {status}");
        let text = String::from_utf8_lossy(&body);
        ensure!(
            text.contains("<html"),
            "index.html must be html document: {text}"
        );
        ensure!(
            header_str(&headers, header::CONTENT_TYPE)?.starts_with("text/html"),
            "unexpected content-type: {:?}",
            headers.get(header::CONTENT_TYPE)
        );
        ensure!(
            header_str(&headers, header::CACHE_CONTROL)?.contains("no-cache"),
            "index.html must not be cached: {:?}",
            headers.get(header::CACHE_CONTROL)
        );

        // 无扩展名路径 → SPA 回退到 index.html（vue-router 接管）。
        let (status, _, fallback) = app.get_raw("/admin/articles/123").await?;
        ensure!(
            status == StatusCode::OK,
            "spa fallback must serve index.html, got {status}"
        );
        ensure!(
            fallback == body,
            "spa fallback must return the same index.html bytes"
        );

        // 带扩展名的缺失资源 → 404（不能把 JS 404 吞成 HTML）。
        let (status, _, _) = app.get_raw("/admin/assets/missing.js").await?;
        ensure!(
            status == StatusCode::NOT_FOUND,
            "missing asset must be 404, got {status}"
        );

        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}
