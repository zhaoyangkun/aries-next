//! HTTP 级集成测试共享 Harness：随机 Schema 隔离 + 真实 Router，无需监听端口。

use anyhow::{Context, bail};
use axum::{
    Router,
    body::Body,
    http::{HeaderMap, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

pub const BOOTSTRAP_SECRET: &str = "aries-test-bootstrap-secret-0123456789";
pub const ADMIN_ORIGIN: &str = "http://test.admin";
pub const OWNER_PASSWORD: &str = "owner-password-1";
pub const MEDIA_PUBLIC_BASE_URL: &str = "/api/media/files";

pub struct TestApp {
    app: Router,
    schema: String,
    admin_pool: PgPool,
    test_pool: PgPool,
    /// 供测试直接驱动后台 Worker 与 Repository（如媒体清理任务的确定性验证）。
    /// 共享 Harness 按测试二进制分别编译，未用到该字段的 Target 不报 Dead Code。
    #[allow(dead_code)]
    pub state: aries_server::state::AppState,
    /// 每个测试实例独立的本地媒体目录，避免跨用例污染。
    #[allow(dead_code)]
    pub media_dir: std::path::PathBuf,
}

pub struct TestResponse {
    pub status: StatusCode,
    pub body: serde_json::Value,
    pub set_cookie: Option<String>,
}

/// Multipart 上传的一个文件字段。
#[allow(dead_code)] // 共享 Harness：仅部分测试 Target 使用
pub struct MultipartFile {
    pub name: String,
    pub filename: String,
    pub content_type: String,
    pub data: Vec<u8>,
}

#[allow(dead_code)] // 共享 Harness：仅部分测试 Target 使用
impl MultipartFile {
    pub fn new(filename: &str, content_type: &str, data: Vec<u8>) -> Self {
        Self {
            name: "file[]".to_owned(),
            filename: filename.to_owned(),
            content_type: content_type.to_owned(),
            data,
        }
    }
}

/// 未设置 `ARIES_RUN_DATABASE_TESTS=1` 时返回 `None`，测试函数直接早退跳过。
#[allow(dead_code)] // 共享 Harness：ai_api 等 Target 只用 maybe_app_with_ai
pub async fn maybe_app() -> anyhow::Result<Option<TestApp>> {
    maybe_app_with_ai(None).await
}

/// 允许注入自定义 AI Provider（如测试用 FakeAiProvider）；None 用真实实现（测试中不会被调用）。
#[allow(dead_code)] // 共享 Harness：仅 ai_api 测试 Target 使用
pub async fn maybe_app_with_ai(
    ai_provider: Option<std::sync::Arc<dyn aries_core::ai::AiProvider>>,
) -> anyhow::Result<Option<TestApp>> {
    if std::env::var("ARIES_RUN_DATABASE_TESTS").as_deref() != Ok("1") {
        return Ok(None);
    }
    let _ = dotenvy::dotenv();
    let base_config = aries_infra::PostgresConfig::from_env()?;
    let admin_pool = aries_infra::connect_postgres(&base_config).await?;
    let test_schema = format!("aries_test_{}", Uuid::now_v7().simple());

    sqlx::query(&format!("CREATE SCHEMA \"{test_schema}\""))
        .execute(&admin_pool)
        .await
        .context("failed to create isolated server test schema")?;
    let test_config =
        base_config.with_search_path(format!("{test_schema},{}", base_config.schema()));
    let test_pool = aries_infra::connect_postgres(&test_config).await?;

    let media_dir =
        std::env::temp_dir().join(format!("aries-test-media-{}", Uuid::now_v7().simple()));

    let setup_result = async {
        aries_infra::run_migrations(&test_pool).await?;
        let config = aries_server::config::ServerConfig {
            address: "127.0.0.1:0".parse()?,
            admin_origins: vec![ADMIN_ORIGIN.to_owned()],
            bootstrap_secret: BOOTSTRAP_SECRET.to_owned(),
            session_ttl: time::Duration::hours(12),
            cookie_secure: false,
            media_public_base_url: MEDIA_PUBLIC_BASE_URL.to_owned(),
            media_provider: "local".to_owned(),
            slow_request_ms: 0,
            log_error_spike_threshold: 0,
            openapi_docs_enabled: false,
        };
        // 与 main.rs 保持一致的组合方式，保证测试覆盖真实的 AppState 组装路径。
        let state = aries_server::state::AppState {
            database: test_pool.clone(),
            auth: std::sync::Arc::new(aries_infra::PostgresAuthRepository::new(test_pool.clone())),
            content: std::sync::Arc::new(aries_infra::PostgresContentRepository::new(
                test_pool.clone(),
            )),
            markdown: std::sync::Arc::new(aries_infra::ComrakMarkdownRenderer),
            passwords: std::sync::Arc::new(aries_infra::Argon2PasswordHasher),
            media: std::sync::Arc::new(aries_infra::PostgresMediaRepository::new(
                test_pool.clone(),
            )),
            storage: std::sync::Arc::new(aries_infra::storage::LocalMediaStorage::new(
                &media_dir,
                MEDIA_PUBLIC_BASE_URL,
            )),
            jobs: std::sync::Arc::new(aries_infra::PostgresJobRepository::new(test_pool.clone())),
            site_settings: std::sync::Arc::new(aries_infra::PostgresSiteSettingsRepository::new(
                test_pool.clone(),
            )),
            comments: std::sync::Arc::new(aries_infra::PostgresCommentRepository::new(
                test_pool.clone(),
            )),
            pages: std::sync::Arc::new(aries_infra::PostgresPageRepository::new(test_pool.clone())),
            journals: std::sync::Arc::new(aries_infra::PostgresJournalRepository::new(
                test_pool.clone(),
            )),
            galleries: std::sync::Arc::new(aries_infra::PostgresGalleryRepository::new(
                test_pool.clone(),
            )),
            links: std::sync::Arc::new(aries_infra::PostgresLinkRepository::new(test_pool.clone())),
            logs: std::sync::Arc::new(aries_infra::PostgresLogRepository::new(test_pool.clone())),
            navigation: std::sync::Arc::new(aries_infra::PostgresNavigationRepository::new(
                test_pool.clone(),
            )),
            settings: std::sync::Arc::new(aries_infra::PostgresSettingRepository::new(
                test_pool.clone(),
            )),
            chunks: std::sync::Arc::new(aries_infra::PostgresChunkRepository::new(
                test_pool.clone(),
            )),
            ai: ai_provider
                .unwrap_or_else(|| std::sync::Arc::new(aries_infra::DispatchingAiProvider::new())),
            ai_requests: std::sync::Arc::new(aries_infra::PostgresAiRequestRepository::new(
                test_pool.clone(),
            )),
            config: std::sync::Arc::new(config),
            rate_limiter: Default::default(),
            log_handle: std::sync::Arc::new(aries_server::logging::LogHandle::noop()),
        };
        let app = aries_server::build_app(state.clone())?;
        Ok::<_, anyhow::Error>((app, state))
    }
    .await;

    match setup_result {
        Ok((app, state)) => Ok(Some(TestApp {
            app,
            schema: test_schema,
            admin_pool,
            test_pool,
            state,
            media_dir,
        })),
        Err(error) => {
            test_pool.close().await;
            let _ = sqlx::query(&format!("DROP SCHEMA \"{test_schema}\" CASCADE"))
                .execute(&admin_pool)
                .await;
            admin_pool.close().await;
            let _ = std::fs::remove_dir_all(&media_dir);
            Err(error)
        }
    }
}

impl TestApp {
    /// 无论测试成功或失败都必须调用，回收随机 Schema 与临时媒体目录。
    pub async fn cleanup(self) -> anyhow::Result<()> {
        self.test_pool.close().await;
        sqlx::query(&format!("DROP SCHEMA \"{}\" CASCADE", self.schema))
            .execute(&self.admin_pool)
            .await
            .context("failed to remove isolated server test schema")?;
        self.admin_pool.close().await;
        let _ = std::fs::remove_dir_all(&self.media_dir);
        Ok(())
    }

    // 共享 Harness：未用到该方法的测试 Target 不报 Dead Code。
    #[allow(dead_code)]
    pub async fn get(&self, path: &str) -> anyhow::Result<TestResponse> {
        self.request("GET", path, None, None).await
    }
}

// 以下 Helper 仅媒体相关测试使用；共享 Harness 在其他测试 Target 中保持静默。
#[allow(dead_code)]
impl TestApp {
    /// Admin 认证 GET：带 Session Cookie，不带 Origin（GET 不过 Origin 校验）。
    pub async fn admin_get(&self, path: &str, cookie: &str) -> anyhow::Result<TestResponse> {
        self.request("GET", path, None, Some(cookie)).await
    }

    /// Admin 写请求：带精确匹配的 `Origin` 与 Session Cookie。
    pub async fn admin_post(
        &self,
        path: &str,
        body: serde_json::Value,
        cookie: Option<&str>,
    ) -> anyhow::Result<TestResponse> {
        self.request("POST", path, Some(body), cookie).await
    }

    pub async fn admin_put(
        &self,
        path: &str,
        body: serde_json::Value,
        cookie: &str,
    ) -> anyhow::Result<TestResponse> {
        self.request("PUT", path, Some(body), Some(cookie)).await
    }

    pub async fn admin_patch(
        &self,
        path: &str,
        body: serde_json::Value,
        cookie: &str,
    ) -> anyhow::Result<TestResponse> {
        self.request("PATCH", path, Some(body), Some(cookie)).await
    }

    pub async fn admin_delete(&self, path: &str, cookie: &str) -> anyhow::Result<TestResponse> {
        let builder = Request::builder()
            .method("DELETE")
            .uri(path)
            .header(header::ORIGIN, ADMIN_ORIGIN)
            .header(header::COOKIE, cookie);
        let response = self
            .app
            .clone()
            .oneshot(builder.body(Body::empty())?)
            .await
            .context("router oneshot failed")?;
        let status = response.status();
        let bytes = response.into_body().collect().await?.to_bytes();
        let body = if bytes.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&bytes).context("response body is not valid JSON")?
        };
        Ok(TestResponse {
            status,
            body,
            set_cookie: None,
        })
    }

    /// 手工拼装 multipart/form-data：字节级构造，覆盖真实文件上传路径。
    pub async fn admin_post_multipart(
        &self,
        path: &str,
        files: Vec<MultipartFile>,
        cookie: &str,
    ) -> anyhow::Result<TestResponse> {
        let boundary = "----aries-test-boundary-0123456789";
        let mut body = Vec::new();
        for file in &files {
            body.extend_from_slice(
                format!(
                    "--{boundary}\r\nContent-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\nContent-Type: {}\r\n\r\n",
                    file.name, file.filename, file.content_type
                )
                .as_bytes(),
            );
            body.extend_from_slice(&file.data);
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());

        let request = Request::builder()
            .method("POST")
            .uri(path)
            .header(
                header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            )
            .header(header::ORIGIN, ADMIN_ORIGIN)
            .header(header::COOKIE, cookie)
            .body(Body::from(body))?;
        let response = self
            .app
            .clone()
            .oneshot(request)
            .await
            .context("router oneshot failed")?;
        let status = response.status();
        let bytes = response.into_body().collect().await?.to_bytes();
        let body = if bytes.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&bytes).context("response body is not valid JSON")?
        };
        Ok(TestResponse {
            status,
            body,
            set_cookie: None,
        })
    }

    /// Public 写端点（access/views）测试：可自定义 Header（如 X-Forwarded-For、Cookie）。
    pub async fn post_with_headers(
        &self,
        path: &str,
        body: serde_json::Value,
        headers: &[(&str, &str)],
    ) -> anyhow::Result<TestResponse> {
        let mut builder = Request::builder()
            .method("POST")
            .uri(path)
            .header(header::CONTENT_TYPE, "application/json");
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        let response = self
            .app
            .clone()
            .oneshot(builder.body(Body::from(serde_json::to_vec(&body)?))?)
            .await
            .context("router oneshot failed")?;
        let status = response.status();
        let set_cookie = response
            .headers()
            .get(header::SET_COOKIE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let bytes = response.into_body().collect().await?.to_bytes();
        let body = if bytes.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&bytes).context("response body is not valid JSON")?
        };
        Ok(TestResponse {
            status,
            body,
            set_cookie,
        })
    }

    /// 读取原始字节响应（静态文件等非 JSON 端点）。
    pub async fn get_raw(&self, path: &str) -> anyhow::Result<(StatusCode, HeaderMap, Vec<u8>)> {
        let response = self
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(path)
                    .body(Body::empty())?,
            )
            .await
            .context("router oneshot failed")?;
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response.into_body().collect().await?.to_bytes().to_vec();
        Ok((status, headers, bytes))
    }

    /// Admin 写请求读取原始字节（SSE 等流式/非 JSON 响应）。
    #[allow(dead_code)] // 共享 Harness：仅 AI 测试 Target 使用
    pub async fn admin_post_raw(
        &self,
        path: &str,
        body: serde_json::Value,
        cookie: &str,
    ) -> anyhow::Result<(StatusCode, HeaderMap, Vec<u8>)> {
        let response = self
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(path)
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::ORIGIN, ADMIN_ORIGIN)
                    .header(header::COOKIE, cookie)
                    .body(Body::from(serde_json::to_vec(&body)?))?,
            )
            .await
            .context("router oneshot failed")?;
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response.into_body().collect().await?.to_bytes().to_vec();
        Ok((status, headers, bytes))
    }

    /// Admin GET 流式响应：返回未收集的 Body（SSE tail 等长连接，由调用方逐帧读取）。
    #[allow(dead_code)] // 共享 Harness：仅 logs 测试 Target 使用
    pub async fn admin_get_streaming(
        &self,
        path: &str,
        cookie: &str,
    ) -> anyhow::Result<(StatusCode, HeaderMap, Body)> {
        let response = self
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(path)
                    .header(header::COOKIE, cookie)
                    .body(Body::empty())?,
            )
            .await
            .context("router oneshot failed")?;
        Ok((
            response.status(),
            response.headers().clone(),
            response.into_body(),
        ))
    }

    async fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<serde_json::Value>,
        cookie: Option<&str>,
    ) -> anyhow::Result<TestResponse> {
        let mut builder = Request::builder().method(method).uri(path);
        let payload = match body {
            Some(value) => {
                builder = builder
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::ORIGIN, ADMIN_ORIGIN);
                Body::from(serde_json::to_vec(&value)?)
            }
            None => Body::empty(),
        };
        if let Some(cookie) = cookie {
            builder = builder.header(header::COOKIE, cookie);
        }
        let response = self
            .app
            .clone()
            .oneshot(builder.body(payload)?)
            .await
            .context("router oneshot failed")?;
        let status = response.status();
        let set_cookie = response
            .headers()
            .get(header::SET_COOKIE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let bytes = response.into_body().collect().await?.to_bytes();
        let body = if bytes.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&bytes).with_context(|| {
                format!(
                    "response {} {} is not valid JSON: {}",
                    status,
                    path,
                    String::from_utf8_lossy(&bytes)
                        .chars()
                        .take(300)
                        .collect::<String>()
                )
            })?
        };
        Ok(TestResponse {
            status,
            body,
            set_cookie,
        })
    }

    /// 走真实的 Bootstrap 端点创建 Owner，并返回 `aries_admin_session=<token>` Cookie 串。
    pub async fn bootstrap_owner(&self) -> anyhow::Result<String> {
        let response = self
            .admin_post(
                "/api/admin/bootstrap",
                serde_json::json!({
                    "bootstrap_secret": BOOTSTRAP_SECRET,
                    "username": "owner",
                    "email": "owner@example.com",
                    "display_name": "Owner",
                    "password": OWNER_PASSWORD,
                }),
                None,
            )
            .await?;
        if response.status != StatusCode::OK {
            bail!("bootstrap failed: {}", response.body);
        }
        let set_cookie = response
            .set_cookie
            .context("bootstrap did not set cookie")?;
        set_cookie
            .split(';')
            .next()
            .map(str::to_owned)
            .context("failed to parse session cookie")
    }
}

/// 惰性连接的 AppState：全部 Repository 为真实 PostgreSQL 实现但连接 lazy（不真正建连），
/// 供不触库的纯 HTTP 层测试（/admin 静态服务、/api/docs 等）构造 Router，
/// 避免在多个测试 Target 重复拼装 AppState。
#[allow(dead_code)] // 共享 Harness：仅部分测试 Target 使用
pub fn lazy_state(config: aries_server::config::ServerConfig) -> aries_server::state::AppState {
    use std::sync::Arc;

    let database = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://test:test@127.0.0.1:1/test")
        .expect("lazy postgres pool");
    aries_server::state::AppState {
        database: database.clone(),
        auth: Arc::new(aries_infra::PostgresAuthRepository::new(database.clone())),
        content: Arc::new(aries_infra::PostgresContentRepository::new(
            database.clone(),
        )),
        markdown: Arc::new(aries_infra::ComrakMarkdownRenderer),
        passwords: Arc::new(aries_infra::Argon2PasswordHasher),
        media: Arc::new(aries_infra::PostgresMediaRepository::new(database.clone())),
        storage: Arc::new(aries_infra::storage::LocalMediaStorage::new(
            std::env::temp_dir(),
            MEDIA_PUBLIC_BASE_URL,
        )),
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
        ai: Arc::new(aries_infra::DispatchingAiProvider::new()),
        ai_requests: Arc::new(aries_infra::PostgresAiRequestRepository::new(database)),
        config: Arc::new(config),
        rate_limiter: Default::default(),
        log_handle: Arc::new(aries_server::logging::LogHandle::noop()),
    }
}
