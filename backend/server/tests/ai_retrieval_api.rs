//! AI 检索（相关文章 + 对话式搜索）的契约测试：
//! smart_search 默认关闭（404）、article_embed 任务管线、相关文章与 ask SSE 事件序列。

mod common;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use anyhow::{Context, ensure};
use async_trait::async_trait;
use axum::http::StatusCode;
use common::TestApp;

use aries_core::ai::{AiChatRequest, AiError, AiProvider, AiSettings, AiStreamEvent, AiUsage};

/// 假 Provider：chat_stream 按脚本回放，embed 返回确定性单位向量（所有 chunk 同向量，
/// 近邻结果只取决于「已发布 + 排除自身」两个过滤条件）。
struct FakeAiProvider {
    stream_scripts: Mutex<VecDeque<Vec<String>>>,
    seen_embed_inputs: Mutex<Vec<String>>,
}

impl FakeAiProvider {
    fn new() -> Self {
        Self {
            stream_scripts: Mutex::new(VecDeque::new()),
            seen_embed_inputs: Mutex::new(Vec::new()),
        }
    }

    fn push_stream(&self, chunks: &[&str]) {
        self.stream_scripts
            .lock()
            .unwrap()
            .push_back(chunks.iter().map(|chunk| (*chunk).to_owned()).collect());
    }
}

#[async_trait]
impl AiProvider for FakeAiProvider {
    async fn chat(
        &self,
        _settings: &AiSettings,
        _request: AiChatRequest,
    ) -> Result<(String, Option<AiUsage>), AiError> {
        Ok((
            "fake".to_owned(),
            Some(AiUsage {
                prompt_tokens: Some(1),
                completion_tokens: Some(1),
            }),
        ))
    }

    async fn chat_stream(
        &self,
        _settings: &AiSettings,
        _request: AiChatRequest,
        sender: tokio::sync::mpsc::Sender<AiStreamEvent>,
    ) -> Result<(), AiError> {
        let chunks = self
            .stream_scripts
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| vec!["fake-answer".to_owned()]);
        for chunk in chunks {
            sender.send(AiStreamEvent::Delta(chunk)).await.unwrap();
        }
        sender
            .send(AiStreamEvent::Usage(AiUsage {
                prompt_tokens: Some(12),
                completion_tokens: Some(7),
            }))
            .await
            .unwrap();
        sender.send(AiStreamEvent::Done).await.unwrap();
        Ok(())
    }

    async fn embed(
        &self,
        _settings: &AiSettings,
        texts: &[String],
    ) -> Result<Vec<Vec<f32>>, AiError> {
        self.seen_embed_inputs
            .lock()
            .unwrap()
            .extend(texts.iter().cloned());
        Ok(texts.iter().map(|_| vec![1.0_f32, 0.0, 0.0]).collect())
    }

    async fn list_models(&self, _settings: &AiSettings) -> Result<Vec<String>, AiError> {
        Ok(vec!["fake-model".to_owned()])
    }
}

/// 解析 SSE 原始字节为 (event, data) 序列。
fn parse_sse(bytes: &[u8]) -> Vec<(String, String)> {
    let text = String::from_utf8_lossy(bytes);
    text.split("\n\n")
        .filter_map(|block| {
            let mut event = None;
            let mut data = None;
            for line in block.lines() {
                if let Some(value) = line.strip_prefix("event:") {
                    event = Some(value.trim().to_owned());
                } else if let Some(value) = line.strip_prefix("data:") {
                    data = Some(value.trim().to_owned());
                }
            }
            event.zip(data)
        })
        .collect()
}

/// 开启 AI 并打开 smart_search + embedding 配置。
async fn configure_smart_search(app: &TestApp, cookie: &str) -> anyhow::Result<()> {
    let current = app.admin_get("/api/admin/settings/ai", cookie).await?;
    ensure!(current.status == StatusCode::OK, "{}", current.body);
    let version = current.body["version"]
        .as_i64()
        .context("missing version")?;
    let updated = app
        .admin_put(
            "/api/admin/settings/ai",
            serde_json::json!({
                "expected_version": version,
                "settings": {
                    "enabled": true,
                    "base_url": "https://fake.example.com/v1",
                    "model": "fake-model",
                    "api_key": "sk-fake",
                    "features": {
                        "editor_assist": false,
                        "comment_moderation": false,
                        "smart_search": true
                    },
                    "embedding_base_url": "https://fake.example.com/v1",
                    "embedding_model": "fake-embedding"
                }
            }),
            cookie,
        )
        .await?;
    ensure!(updated.status == StatusCode::OK, "{}", updated.body);
    Ok(())
}

/// 创建并发布文章，返回 (id, slug)。
async fn create_published_article(
    app: &TestApp,
    cookie: &str,
    title: &str,
    markdown: &str,
) -> anyhow::Result<(i64, String)> {
    let created = app
        .admin_post(
            "/api/admin/articles",
            serde_json::json!({ "title": title, "markdown_source": markdown }),
            Some(cookie),
        )
        .await?;
    ensure!(created.status == StatusCode::CREATED, "{}", created.body);
    let id = created.body["id"].as_i64().context("missing id")?;
    let version = created.body["version"]
        .as_i64()
        .context("missing version")?;
    let slug = created.body["slug"]
        .as_str()
        .context("missing slug")?
        .to_owned();
    let published = app
        .admin_patch(
            &format!("/api/admin/articles/{id}/status"),
            serde_json::json!({ "command": "publish", "expected_version": version }),
            cookie,
        )
        .await?;
    ensure!(published.status == StatusCode::OK, "{}", published.body);
    Ok((id, slug))
}

#[tokio::test]
async fn retrieval_endpoints_hidden_when_smart_search_off() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;
        let (_id, slug) =
            create_published_article(&app, &owner_cookie, "Hidden Post", "# c").await?;

        let related = app
            .get(&format!("/api/public/articles/{slug}/related"))
            .await?;
        ensure!(
            related.status == StatusCode::NOT_FOUND,
            "expected 404, got {}: {}",
            related.status,
            related.body
        );
        ensure!(
            related.body["error"]["code"] == "AI_RETRIEVAL_DISABLED",
            "unexpected error code: {}",
            related.body
        );

        let ask = app
            .post_with_headers(
                "/api/public/search/ask",
                serde_json::json!({ "question": "这篇讲了什么" }),
                &[],
            )
            .await?;
        ensure!(
            ask.status == StatusCode::NOT_FOUND,
            "expected 404, got {}: {}",
            ask.status,
            ask.body
        );
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn article_embed_pipeline_feeds_related_articles() -> anyhow::Result<()> {
    let provider = Arc::new(FakeAiProvider::new());
    let Some(app) = common::maybe_app_with_ai(Some(provider.clone())).await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;
        configure_smart_search(&app, &owner_cookie).await?;

        let (rust_id, rust_slug) = create_published_article(
            &app,
            &owner_cookie,
            "Rust 所有权笔记",
            "# 所有权\n\nRust 的所有权系统。\n\n# 借用\n\n借用与生命周期。",
        )
        .await?;
        let (vue_id, vue_slug) = create_published_article(
            &app,
            &owner_cookie,
            "Vue 响应式笔记",
            "# 响应式\n\nVue 的响应式原理。",
        )
        .await?;

        // 发布触发 article_embed 任务；驱动 worker 完成向量化入库。
        aries_server::worker::run_pending_jobs(&app.state).await;

        let chunk_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM article_chunks WHERE article_id = $1")
                .bind(rust_id)
                .fetch_one(&app.state.database)
                .await?;
        ensure!(
            chunk_count >= 2,
            "expected chunks for rust article, got {chunk_count}"
        );

        let related = app
            .get(&format!("/api/public/articles/{rust_slug}/related"))
            .await?;
        ensure!(related.status == StatusCode::OK, "{}", related.body);
        let items = related
            .body
            .as_array()
            .context("related body is not an array")?;
        ensure!(
            items.iter().any(|item| item["slug"] == vue_slug),
            "expected vue article in related: {}",
            related.body
        );
        ensure!(
            items.iter().all(|item| item["slug"] != rust_slug),
            "related must exclude the article itself: {}",
            related.body
        );

        // 取消发布（回收）后重跑任务：chunks 被清除，相关文章不再包含它。
        let detail = app
            .admin_get(&format!("/api/admin/articles/{vue_id}"), &owner_cookie)
            .await?;
        let vue_version = detail.body["version"].as_i64().context("missing version")?;
        let recycled = app
            .admin_patch(
                &format!("/api/admin/articles/{vue_id}/status"),
                serde_json::json!({ "command": "recycle", "expected_version": vue_version }),
                &owner_cookie,
            )
            .await?;
        ensure!(recycled.status == StatusCode::OK, "{}", recycled.body);
        aries_server::worker::run_pending_jobs(&app.state).await;

        let related_after = app
            .get(&format!("/api/public/articles/{rust_slug}/related"))
            .await?;
        ensure!(
            related_after.status == StatusCode::OK,
            "{}",
            related_after.body
        );
        ensure!(
            related_after
                .body
                .as_array()
                .context("not array")?
                .is_empty(),
            "expected empty related after unpublish: {}",
            related_after.body
        );
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn search_ask_streams_answer_and_sources() -> anyhow::Result<()> {
    let provider = Arc::new(FakeAiProvider::new());
    let Some(app) = common::maybe_app_with_ai(Some(provider.clone())).await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;
        configure_smart_search(&app, &owner_cookie).await?;
        create_published_article(
            &app,
            &owner_cookie,
            "Rust 所有权笔记",
            "# 所有权\n\nRust 的所有权系统保证内存安全。",
        )
        .await?;
        aries_server::worker::run_pending_jobs(&app.state).await;

        provider.push_stream(&["站内有相关内容", "：所有权系统保证内存安全。"]);
        let (status, _headers, bytes) = app
            .admin_post_raw(
                "/api/public/search/ask",
                serde_json::json!({ "question": "Rust 如何保证内存安全" }),
                &owner_cookie,
            )
            .await?;
        ensure!(status == StatusCode::OK, "ask failed: {}", status);

        let events = parse_sse(&bytes);
        let names: Vec<&str> = events.iter().map(|(name, _)| name.as_str()).collect();
        ensure!(
            names.first() == Some(&"start"),
            "first event must be start: {names:?}"
        );
        ensure!(
            names.contains(&"delta"),
            "stream must contain delta: {names:?}"
        );
        ensure!(
            names.contains(&"sources"),
            "stream must contain sources: {names:?}"
        );
        ensure!(
            names.last() == Some(&"done"),
            "last event must be done: {names:?}"
        );

        let sources = events
            .iter()
            .find(|(name, _)| name == "sources")
            .and_then(|(_, data)| serde_json::from_str::<serde_json::Value>(data).ok())
            .context("missing sources payload")?;
        ensure!(
            sources["items"]
                .as_array()
                .context("items missing")?
                .iter()
                .any(|item| item["title"] == "Rust 所有权笔记"),
            "sources must cite the rust article: {sources}"
        );

        // 审计落库：search_ask 记录存在。
        let audited: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM ai_requests WHERE feature = 'search_ask' AND status = 'success'",
        )
        .fetch_one(&app.state.database)
        .await?;
        ensure!(audited >= 1, "expected search_ask audit record");
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}
