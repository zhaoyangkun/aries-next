//! Phase 08 第一批 Contract Test：编辑器助手 SSE / 用量审计 / 评论 AI 审核挂钩。
//! 注入 FakeAiProvider，不触网。

mod common;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use anyhow::{Context, ensure};
use async_trait::async_trait;
use axum::http::StatusCode;

use aries_core::ai::{AiChatRequest, AiError, AiProvider, AiSettings, AiStreamEvent, AiUsage};
use common::TestApp;

/// 假 Provider：chat 按脚本队列返回，chat_stream 逐段回放增量。
struct FakeAiProvider {
    chat_scripts: Mutex<VecDeque<Result<String, AiError>>>,
    stream_scripts: Mutex<VecDeque<Vec<String>>>,
    /// 记录收到的全部用户输入，验证 Prompt 包裹行为。
    seen_inputs: Mutex<Vec<String>>,
}

impl FakeAiProvider {
    fn new() -> Self {
        Self {
            chat_scripts: Mutex::new(VecDeque::new()),
            stream_scripts: Mutex::new(VecDeque::new()),
            seen_inputs: Mutex::new(Vec::new()),
        }
    }

    fn push_chat(&self, script: Result<&str, AiError>) {
        self.chat_scripts
            .lock()
            .unwrap()
            .push_back(script.map(str::to_owned));
    }

    fn push_stream(&self, chunks: &[&str]) {
        self.stream_scripts
            .lock()
            .unwrap()
            .push_back(chunks.iter().map(|s| s.to_string()).collect());
    }
}

#[async_trait]
impl AiProvider for FakeAiProvider {
    async fn chat(
        &self,
        _settings: &AiSettings,
        request: AiChatRequest,
    ) -> Result<(String, Option<AiUsage>), AiError> {
        for message in &request.messages {
            self.seen_inputs
                .lock()
                .unwrap()
                .push(message.content.clone());
        }
        let script = self
            .chat_scripts
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Ok("default-fake".to_owned()));
        script.map(|text| {
            (
                text,
                Some(AiUsage {
                    prompt_tokens: Some(10),
                    completion_tokens: Some(5),
                }),
            )
        })
    }

    async fn chat_stream(
        &self,
        _settings: &AiSettings,
        request: AiChatRequest,
        sender: tokio::sync::mpsc::Sender<AiStreamEvent>,
    ) -> Result<(), AiError> {
        for message in &request.messages {
            self.seen_inputs
                .lock()
                .unwrap()
                .push(message.content.clone());
        }
        let chunks = self
            .stream_scripts
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| vec!["fake".to_owned()]);
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
        self.seen_inputs
            .lock()
            .unwrap()
            .extend(texts.iter().cloned());
        Ok(texts.iter().map(|_| vec![1.0_f32, 0.0, 0.0]).collect())
    }

    async fn list_models(&self, _settings: &AiSettings) -> Result<Vec<String>, AiError> {
        Ok(vec!["fake-model-a".to_owned(), "fake-model-b".to_owned()])
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

/// 「获取模型」：已保存 api_key 即可拉取（不要求启用总开关），未配置返回 400。
#[tokio::test]
async fn list_ai_models_requires_saved_api_key_but_not_enabled() -> anyhow::Result<()> {
    let provider = Arc::new(FakeAiProvider::new());
    let Some(app) = common::maybe_app_with_ai(Some(provider.clone())).await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;

        // 未配置 api_key → 400 AI_NOT_CONFIGURED。
        let unconfigured = app
            .admin_get("/api/admin/ai/models", &owner_cookie)
            .await?;
        ensure!(
            unconfigured.status == StatusCode::BAD_REQUEST,
            "{}",
            unconfigured.body
        );
        ensure!(
            unconfigured.body["error"]["code"] == "AI_NOT_CONFIGURED",
            "{}",
            unconfigured.body
        );

        // 只保存 api_key 与 base_url、不启用总开关 → 200。
        let current = app.admin_get("/api/admin/settings/ai", &owner_cookie).await?;
        let version = current.body["version"].as_i64().context("missing version")?;
        let updated = app
            .admin_put(
                "/api/admin/settings/ai",
                serde_json::json!({
                    "expected_version": version,
                    "settings": {
                        "base_url": "https://fake.example.com/v1",
                        "api_key": "sk-fake"
                    }
                }),
                &owner_cookie,
            )
            .await?;
        ensure!(updated.status == StatusCode::OK, "{}", updated.body);

        let listed = app.admin_get("/api/admin/ai/models", &owner_cookie).await?;
        ensure!(listed.status == StatusCode::OK, "{}", listed.body);
        let models = listed.body["models"]
            .as_array()
            .context("missing models")?;
        ensure!(
            models.iter().any(|model| model == "fake-model-a"),
            "{}",
            listed.body
        );

        // editor（无 ManageSettings）→ 403。
        let password_hash = app.state.passwords.hash("editor-pass-1")?;
        sqlx::query(
            "INSERT INTO users (username, email, password_hash, display_name, role, status) \
             VALUES ('models-editor', 'models-editor@example.com', $1, 'Editor', 'editor', 'active')",
        )
        .bind(&password_hash)
        .execute(&app.state.database)
        .await?;
        let login = app
            .admin_post(
                "/api/admin/auth/login",
                serde_json::json!({ "login": "models-editor", "password": "editor-pass-1" }),
                None,
            )
            .await?;
        ensure!(login.status == StatusCode::OK, "{}", login.body);
        let editor_cookie = login
            .set_cookie
            .context("login did not set cookie")?
            .split(';')
            .next()
            .context("failed to parse session cookie")?
            .to_owned();
        let forbidden = app.admin_get("/api/admin/ai/models", &editor_cookie).await?;
        ensure!(
            forbidden.status == StatusCode::FORBIDDEN,
            "{}",
            forbidden.body
        );
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

/// 开启 AI 设置（editor_assist 与 comment_moderation 按需）。
async fn configure_ai(
    app: &TestApp,
    cookie: &str,
    editor: bool,
    moderation: bool,
) -> anyhow::Result<()> {
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
                    "features": { "editor_assist": editor, "comment_moderation": moderation }
                }
            }),
            cookie,
        )
        .await?;
    ensure!(updated.status == StatusCode::OK, "{}", updated.body);
    Ok(())
}

async fn create_published_article(app: &TestApp, cookie: &str) -> anyhow::Result<String> {
    let created = app
        .admin_post(
            "/api/admin/articles",
            serde_json::json!({ "title": "AI Post", "markdown_source": "# c" }),
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
    Ok(slug)
}

#[tokio::test]
async fn editor_rewrite_streams_sse_and_records_usage() -> anyhow::Result<()> {
    let provider = Arc::new(FakeAiProvider::new());
    provider.push_stream(&["Hello", " world"]);
    let Some(app) = common::maybe_app_with_ai(Some(provider.clone())).await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;
        configure_ai(&app, &owner_cookie, true, false).await?;

        // ai 组 GET：api_key 不回明文。
        let settings = app
            .admin_get("/api/admin/settings/ai", &owner_cookie)
            .await?;
        ensure!(settings.body["settings"]["api_key_set"] == true);
        ensure!(settings.body["settings"].get("api_key").is_none());

        let (status, headers, bytes) = app
            .admin_post_raw(
                "/api/admin/ai/editor/rewrite",
                serde_json::json!({ "text": "hello" }),
                &owner_cookie,
            )
            .await?;
        ensure!(status == StatusCode::OK);
        let content_type = headers
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .context("missing content-type")?;
        ensure!(
            content_type.starts_with("text/event-stream"),
            "{content_type}"
        );

        // SSE 事件序列：start → delta × 2 → usage → done。
        let events = parse_sse(&bytes);
        let names: Vec<&str> = events.iter().map(|(name, _)| name.as_str()).collect();
        ensure!(
            names == vec!["start", "delta", "delta", "usage", "done"],
            "{names:?}"
        );
        ensure!(events[0].1.contains("editor_rewrite"));
        ensure!(events[0].1.contains("fake-model"));
        ensure!(events[1].1.contains("Hello"));
        ensure!(events[2].1.contains(" world"));
        ensure!(events[3].1.contains("prompt_tokens"));

        // Prompt 中的用户输入被不可信边界包裹（注入防护）。
        let seen = provider.seen_inputs.lock().unwrap().clone();
        ensure!(
            seen.iter()
                .any(|input| input.contains("UNTRUSTED") && input.contains("hello")),
            "{seen:?}"
        );

        // ai_requests 落库：success + token 用量。
        let (recorded,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM ai_requests \
             WHERE feature = 'editor_rewrite' AND status = 'success' \
             AND prompt_tokens = 12 AND completion_tokens = 7",
        )
        .fetch_one(&app.state.database)
        .await?;
        ensure!(recorded == 1);

        // Audit：ai.editor_rewrite，不含完整 Prompt。
        let (audited,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM audit_logs WHERE action = 'ai.editor_rewrite'")
                .fetch_one(&app.state.database)
                .await?;
        ensure!(audited == 1);

        // 用量查询端点（ManageSettings）。
        let usage = app
            .admin_get("/api/admin/ai/usage?feature=editor_rewrite", &owner_cookie)
            .await?;
        ensure!(usage.status == StatusCode::OK, "{}", usage.body);
        ensure!(usage.body["total"].as_i64() == Some(1));
        ensure!(usage.body["items"][0]["model"] == "fake-model");
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn editor_assist_guards_disabled_unconfigured_and_toggle() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app_with_ai(Some(Arc::new(FakeAiProvider::new()))).await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;

        // 默认未启用 → 403 AI_DISABLED。
        let disabled = app
            .admin_post(
                "/api/admin/ai/editor/rewrite",
                serde_json::json!({ "text": "hi" }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(
            disabled.status == StatusCode::FORBIDDEN,
            "{}",
            disabled.body
        );
        ensure!(disabled.body["error"]["code"] == "AI_DISABLED");

        // 启用但缺配置（无 base_url/model/api_key）→ 400 AI_NOT_CONFIGURED。
        let current = app
            .admin_get("/api/admin/settings/ai", &owner_cookie)
            .await?;
        let version = current.body["version"]
            .as_i64()
            .context("missing version")?;
        let partial = app
            .admin_put(
                "/api/admin/settings/ai",
                serde_json::json!({
                    "expected_version": version,
                    "settings": { "enabled": true }
                }),
                &owner_cookie,
            )
            .await?;
        ensure!(partial.status == StatusCode::OK, "{}", partial.body);
        let unconfigured = app
            .admin_post(
                "/api/admin/ai/editor/rewrite",
                serde_json::json!({ "text": "hi" }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(
            unconfigured.status == StatusCode::BAD_REQUEST,
            "{}",
            unconfigured.body
        );
        ensure!(unconfigured.body["error"]["code"] == "AI_NOT_CONFIGURED");

        // 配置齐全但 editor_assist 开关关闭 → 403 AI_FEATURE_DISABLED。
        configure_ai(&app, &owner_cookie, false, true).await?;
        let toggle_off = app
            .admin_post(
                "/api/admin/ai/editor/rewrite",
                serde_json::json!({ "text": "hi" }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(
            toggle_off.status == StatusCode::FORBIDDEN,
            "{}",
            toggle_off.body
        );
        ensure!(toggle_off.body["error"]["code"] == "AI_FEATURE_DISABLED");

        // 空输入 → 400。
        let empty = app
            .admin_post(
                "/api/admin/ai/editor/rewrite",
                serde_json::json!({ "text": "  " }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(empty.status == StatusCode::BAD_REQUEST, "{}", empty.body);

        // 权限：editor（ManageContent）可用编辑器端点；usage 查询需 ManageSettings。
        let password_hash = app.state.passwords.hash("editor-pass-1")?;
        sqlx::query(
            "INSERT INTO users (username, email, password_hash, display_name, role, status) \
             VALUES ('editor-ai', 'editor-ai@example.com', $1, 'Editor', 'editor', 'active')",
        )
        .bind(&password_hash)
        .execute(&app.state.database)
        .await?;
        let login = app
            .admin_post(
                "/api/admin/auth/login",
                serde_json::json!({ "login": "editor-ai", "password": "editor-pass-1" }),
                None,
            )
            .await?;
        ensure!(login.status == StatusCode::OK, "{}", login.body);
        let editor_cookie = login
            .set_cookie
            .context("login did not set cookie")?
            .split(';')
            .next()
            .context("failed to parse session cookie")?
            .to_owned();

        // editor 访问 usage 查询 → 403（ManageSettings 仅 owner）。
        let usage_denied = app.admin_get("/api/admin/ai/usage", &editor_cookie).await?;
        ensure!(
            usage_denied.status == StatusCode::FORBIDDEN,
            "{}",
            usage_denied.body
        );

        // 未认证 → 401。
        let unauthenticated = app.get("/api/admin/ai/usage").await?;
        ensure!(unauthenticated.status == StatusCode::UNAUTHORIZED);
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn editor_metadata_rejects_non_json_output() -> anyhow::Result<()> {
    let provider = Arc::new(FakeAiProvider::new());
    provider.push_stream(&["not json at all"]);
    let Some(app) = common::maybe_app_with_ai(Some(provider)).await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;
        configure_ai(&app, &owner_cookie, true, false).await?;

        let (status, _headers, bytes) = app
            .admin_post_raw(
                "/api/admin/ai/editor/metadata",
                serde_json::json!({ "title": "t", "content": "c" }),
                &owner_cookie,
            )
            .await?;
        ensure!(status == StatusCode::OK);
        let events = parse_sse(&bytes);
        let names: Vec<&str> = events.iter().map(|(name, _)| name.as_str()).collect();
        // 输出不是合法 JSON：delta 之后是 error 而不是 done。
        ensure!(names.last() == Some(&"error"), "{names:?}");
        ensure!(events.last().map(|(_, data)| data.contains("AI_INVALID_OUTPUT")) == Some(true));

        let (failed,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM ai_requests \
             WHERE feature = 'editor_metadata' AND status = 'failed' AND error_category = 'invalid_output'",
        )
        .fetch_one(&app.state.database)
        .await?;
        ensure!(failed == 1);
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn comment_moderation_marks_spam_and_falls_back_on_failure() -> anyhow::Result<()> {
    let provider = Arc::new(FakeAiProvider::new());
    // 三条评论的审核脚本：高置信 SPAM、SAFE、Provider 失败。
    provider.push_chat(Ok(r#"{"risk":"spam","reason":"广告","confidence":0.95}"#));
    provider.push_chat(Ok(r#"{"risk":"safe","reason":"正常","confidence":0.99}"#));
    provider.push_chat(Err(AiError::ProviderFailed));
    let Some(app) = common::maybe_app_with_ai(Some(provider)).await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;
        configure_ai(&app, &owner_cookie, false, true).await?;
        let slug = create_published_article(&app, &owner_cookie).await?;

        async fn submit(
            app: &TestApp,
            slug: &str,
            ip: &str,
            content: &str,
        ) -> anyhow::Result<common::TestResponse> {
            app.post_with_headers(
                "/api/public/comments",
                serde_json::json!({
                    "target_type": "article", "target_slug": slug,
                    "nickname": "Reader", "email": format!("{ip}@example.com"),
                    "content": content,
                }),
                &[("x-forwarded-for", ip)],
            )
            .await
        }

        // 高置信 SPAM → 状态直接变 spam，且写入 ai_* 结论。
        let spam = submit(&app, &slug, "10.40.0.1", "buy cheap stuff").await?;
        ensure!(spam.status == StatusCode::OK, "{}", spam.body);
        ensure!(spam.body["status"] == "spam", "{}", spam.body);
        let spam_id = spam.body["id"].as_i64().context("missing id")?;
        let (risk, reason): (Option<String>, Option<String>) = sqlx::query_as(
            "SELECT ai_risk, ai_reason FROM comments WHERE id = $1",
        )
        .bind(spam_id)
        .fetch_one(&app.state.database)
        .await?;
        ensure!(risk.as_deref() == Some("spam"));
        ensure!(reason.as_deref() == Some("广告"));

        // SAFE → 保持原策略状态（moderated → pending），记录 ai_risk = safe。
        let safe = submit(&app, &slug, "10.40.0.2", "great article").await?;
        ensure!(safe.status == StatusCode::OK, "{}", safe.body);
        ensure!(safe.body["status"] == "pending", "{}", safe.body);
        let safe_id = safe.body["id"].as_i64().context("missing id")?;
        let (risk,): (Option<String>,) =
            sqlx::query_as("SELECT ai_risk FROM comments WHERE id = $1")
                .bind(safe_id)
                .fetch_one(&app.state.database)
                .await?;
        ensure!(risk.as_deref() == Some("safe"));

        // Provider 失败 → 静默回退：评论照常提交为 pending，无 ai_risk。
        let fallback = submit(&app, &slug, "10.40.0.3", "no ai today").await?;
        ensure!(fallback.status == StatusCode::OK, "{}", fallback.body);
        ensure!(fallback.body["status"] == "pending", "{}", fallback.body);
        let fallback_id = fallback.body["id"].as_i64().context("missing id")?;
        let (risk,): (Option<String>,) =
            sqlx::query_as("SELECT ai_risk FROM comments WHERE id = $1")
                .bind(fallback_id)
                .fetch_one(&app.state.database)
                .await?;
        ensure!(risk.is_none());

        // 审核请求全部落审计：success ×2 + failed ×1，operator 为空（系统触发）。
        let (moderation_rows,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM ai_requests WHERE feature = 'comment_moderation'",
        )
        .fetch_one(&app.state.database)
        .await?;
        ensure!(moderation_rows == 3);
        let (failed_rows,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM ai_requests \
             WHERE feature = 'comment_moderation' AND status = 'failed' AND operator_user_id IS NULL",
        )
        .fetch_one(&app.state.database)
        .await?;
        ensure!(failed_rows == 1);

        // spam 评论不出现在公开列表；pending 的也不出现。
        let list = app
            .get(&format!("/api/public/articles/{slug}/comments"))
            .await?;
        ensure!(list.status == StatusCode::OK, "{}", list.body);
        ensure!(list.body["total"].as_i64() == Some(0));
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn editor_tags_validates_json_and_editor_brief_streams_text() -> anyhow::Result<()> {
    let provider = Arc::new(FakeAiProvider::new());
    provider.push_stream(&["{\"tags\": [\"Rust\", \"所有权\"]}"]);
    provider.push_stream(&["这是导读", "内容。"]);
    let Some(app) = common::maybe_app_with_ai(Some(provider)).await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;
        configure_ai(&app, &owner_cookie, true, false).await?;

        // tags：JSON 合法 → done；事件序列 start → delta → usage → done。
        let (_status, _headers, bytes) = app
            .admin_post_raw(
                "/api/admin/ai/editor/tags",
                serde_json::json!({ "title": "t", "content": "c" }),
                &owner_cookie,
            )
            .await?;
        let events = parse_sse(&bytes);
        let names: Vec<&str> = events.iter().map(|(name, _)| name.as_str()).collect();
        ensure!(
            names == vec!["start", "delta", "usage", "done"],
            "tags events: {names:?}"
        );
        ensure!(events[0].1.contains("editor_tags"));
        ensure!(events[1].1.contains("Rust"));

        // brief：纯文本 → done，无需 JSON 校验。
        let (_status, _headers, bytes) = app
            .admin_post_raw(
                "/api/admin/ai/editor/brief",
                serde_json::json!({ "title": "t", "content": "c" }),
                &owner_cookie,
            )
            .await?;
        let events = parse_sse(&bytes);
        let names: Vec<&str> = events.iter().map(|(name, _)| name.as_str()).collect();
        ensure!(
            names == vec!["start", "delta", "delta", "usage", "done"],
            "brief events: {names:?}"
        );
        ensure!(events[0].1.contains("ai_brief"));
        let (recorded,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM ai_requests \
             WHERE feature IN ('editor_tags', 'ai_brief') AND status = 'success'",
        )
        .fetch_one(&app.state.database)
        .await?;
        ensure!(recorded == 2);
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn editor_tags_rejects_non_json_output() -> anyhow::Result<()> {
    let provider = Arc::new(FakeAiProvider::new());
    provider.push_stream(&["Rust, 所有权"]);
    let Some(app) = common::maybe_app_with_ai(Some(provider)).await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;
        configure_ai(&app, &owner_cookie, true, false).await?;

        let (_status, _headers, bytes) = app
            .admin_post_raw(
                "/api/admin/ai/editor/tags",
                serde_json::json!({ "title": "t", "content": "c" }),
                &owner_cookie,
            )
            .await?;
        let events = parse_sse(&bytes);
        ensure!(
            events.last().map(|(name, _)| name.as_str()) == Some("error"),
            "{events:?}"
        );
        let (failed,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM ai_requests \
             WHERE feature = 'editor_tags' AND status = 'failed' AND error_category = 'invalid_output'",
        )
        .fetch_one(&app.state.database)
        .await?;
        ensure!(failed == 1);
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}
