//! OpenAPI 文档挂载契约测试：OPENAPI_DOCS 开关联动 /api/docs 的 200/404。
//! docs 路由只返回静态内容，不触库；AppState 用 lazy 连接（不会真正建连）。

use std::sync::Arc;

use async_trait::async_trait;
use axum::{body::Body, http::StatusCode};
use http_body_util::BodyExt;
use sqlx::postgres::PgPoolOptions;
use tower::ServiceExt;

use aries_core::media::{MediaError, MediaStorage};
use aries_infra::DispatchingAiProvider;
use aries_server::build_app;
use aries_server::config::ServerConfig;
use aries_server::rate_limit::RateLimiter;
use aries_server::state::AppState;

struct StubMediaStorage;

#[async_trait]
impl MediaStorage for StubMediaStorage {
    async fn put(&self, _key: &str, _bytes: &[u8], _mime: &str) -> Result<String, MediaError> {
        Err(MediaError::StoreUnavailable)
    }

    async fn get(&self, _key: &str) -> Result<Option<Vec<u8>>, MediaError> {
        Ok(None)
    }

    async fn delete(&self, _key: &str) -> Result<(), MediaError> {
        Ok(())
    }

    fn url_for(&self, key: &str) -> String {
        format!("/api/media/files/{key}")
    }

    fn serves_files_locally(&self) -> bool {
        true
    }
}

fn test_app(openapi_docs_enabled: bool) -> axum::Router {
    let database = PgPoolOptions::new()
        .connect_lazy("postgres://test:test@127.0.0.1:1/test")
        .unwrap();
    build_app(AppState {
        database: database.clone(),
        auth: Arc::new(aries_infra::PostgresAuthRepository::new(database.clone())),
        content: Arc::new(aries_infra::PostgresContentRepository::new(
            database.clone(),
        )),
        markdown: Arc::new(aries_infra::ComrakMarkdownRenderer),
        passwords: Arc::new(aries_infra::Argon2PasswordHasher),
        media: Arc::new(aries_infra::PostgresMediaRepository::new(database.clone())),
        storage: Arc::new(StubMediaStorage),
        jobs: Arc::new(aries_infra::PostgresJobRepository::new(database.clone())),
        site_settings: Arc::new(aries_infra::PostgresSiteSettingsRepository::new(
            database.clone(),
        )),
        comments: Arc::new(aries_infra::PostgresCommentRepository::new(
            database.clone(),
        )),
        pages: Arc::new(aries_infra::PostgresPageRepository::new(database.clone())),
        journals: Arc::new(aries_infra::PostgresJournalRepository::new(
            database.clone(),
        )),
        galleries: Arc::new(aries_infra::PostgresGalleryRepository::new(
            database.clone(),
        )),
        links: Arc::new(aries_infra::PostgresLinkRepository::new(database.clone())),
        logs: Arc::new(aries_infra::PostgresLogRepository::new(database.clone())),
        navigation: Arc::new(aries_infra::PostgresNavigationRepository::new(
            database.clone(),
        )),
        settings: Arc::new(aries_infra::PostgresSettingRepository::new(
            database.clone(),
        )),
        chunks: Arc::new(aries_infra::PostgresChunkRepository::new(database.clone())),
        ai: Arc::new(DispatchingAiProvider::new()),
        ai_requests: Arc::new(aries_infra::PostgresAiRequestRepository::new(database)),
        config: Arc::new(ServerConfig {
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
        }),
        rate_limiter: RateLimiter::default(),
        log_handle: Arc::new(aries_server::logging::LogHandle::noop()),
    })
    .unwrap()
}

async fn get_body(app: axum::Router, path: &str) -> (StatusCode, String) {
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
    (status, body)
}

#[tokio::test]
async fn openapi_docs_mounts_only_when_enabled() {
    // 开启：/api/docs 返回内嵌 spec 的 Scalar HTML，包含已迁移端点与安全方案。
    let (status, body) = get_body(test_app(true), "/api/docs").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("/api/admin/ai/models"),
        "spec missing pilot path"
    );
    assert!(
        body.contains("aries_admin_session"),
        "spec missing security scheme"
    );

    // 关闭（默认）：不暴露文档端点的存在。
    let (status, _) = get_body(test_app(false), "/api/docs").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[test]
fn exported_yaml_matches_committed_generated_file() {
    // 漂移守护的本地等价物：CI 跑同一命令与 docs/openapi.generated.yaml 逐字节 diff。
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_openapi-export"))
        .output()
        .expect("run openapi-export");
    assert!(output.status.success());
    let generated = String::from_utf8(output.stdout).unwrap();
    let committed = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/openapi.generated.yaml"
    ))
    .expect("read committed docs/openapi.generated.yaml");
    assert_eq!(
        generated, committed,
        "docs/openapi.generated.yaml 与代码注解脱节：请运行 \
         `cargo run -p aries-server --bin openapi-export > docs/openapi.generated.yaml` 并提交"
    );
}
