//! OpenAPI 文档挂载契约测试：OPENAPI_DOCS 开关联动 /api/docs 的 200/404。
//! docs 路由只返回静态内容，不触库；AppState 经 common::lazy_state 惰性连接（不会真正建连）。

#[allow(dead_code)] // 本 Target 只使用 lazy_state，共享 Harness 其余项在此为 Dead Code
mod common;

use axum::{body::Body, http::StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use aries_server::build_app;
use aries_server::config::ServerConfig;

fn test_app(openapi_docs_enabled: bool) -> axum::Router {
    build_app(common::lazy_state(ServerConfig {
        address: "127.0.0.1:0".parse().unwrap(),
        admin_origins: vec!["http://127.0.0.1:5173".to_owned()],
        bootstrap_secret: "test-bootstrap-secret-2026".to_owned(),
        session_ttl: time::Duration::hours(12),
        cookie_secure: false,
        media_public_base_url: "/api/media/files".to_owned(),
        media_provider: "local".to_owned(),
        slow_request_ms: 0,
        log_error_spike_threshold: 0,
        openapi_docs_enabled,
    }))
    .unwrap()
}

async fn get(app: axum::Router, path: &str) -> (StatusCode, axum::http::HeaderMap, String) {
    let response = app
        .oneshot(
            axum::http::Request::builder()
                .method("GET")
                .uri(path)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = String::from_utf8(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    (status, headers, body)
}

#[tokio::test]
async fn openapi_docs_mounts_only_when_enabled() {
    // 开启：/api/docs 返回内嵌 spec 的 Scalar HTML，包含已迁移端点与安全方案；
    // 与其他路由一样经过安全响应头中间件。
    let (status, headers, body) = get(test_app(true), "/api/docs").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("/api/admin/ai/models"),
        "spec missing pilot path"
    );
    assert!(
        body.contains("aries_admin_session"),
        "spec missing security scheme"
    );
    assert_eq!(
        headers
            .get(axum::http::header::X_CONTENT_TYPE_OPTIONS)
            .and_then(|value| value.to_str().ok()),
        Some("nosniff"),
        "docs route must go through security headers middleware"
    );

    // 关闭（默认）：不暴露文档端点的存在。
    let (status, _, _) = get(test_app(false), "/api/docs").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[test]
fn exported_yaml_matches_committed_generated_file() {
    // 漂移守护的本地等价物：CI 跑同一命令与 docs/openapi.yaml 逐字节 diff。
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_openapi-export"))
        .output()
        .expect("run openapi-export");
    assert!(output.status.success());
    let generated = String::from_utf8(output.stdout).unwrap();
    let committed = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/openapi.yaml"
    ))
    .expect("read committed docs/openapi.yaml");
    assert_eq!(
        generated, committed,
        "docs/openapi.yaml 与代码注解脱节：请运行 \
         `cargo run -p aries-server --bin openapi-export > docs/openapi.yaml` 并提交"
    );
}
