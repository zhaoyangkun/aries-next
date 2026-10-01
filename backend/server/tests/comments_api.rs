//! Phase 05 Admin 第一批 Contract Test：评论管理 / Dashboard 聚合 / Audit Log 查询。
//! 通过 `ARIES_RUN_DATABASE_TESTS=1` 门控，随机 Schema 隔离，走真实 Router + PostgreSQL。

mod common;

use anyhow::{Context, ensure};
use axum::http::StatusCode;

use common::TestApp;

/// 直接造一个指定 Role 的用户（生成真实 Argon2 Hash 入库），返回其 id。
async fn create_user(
    app: &TestApp,
    username: &str,
    role: &str,
    password: &str,
) -> anyhow::Result<i64> {
    let password_hash = app
        .state
        .passwords
        .hash(password)
        .context("failed to hash test password")?;
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO users (username, email, password_hash, display_name, role, status) \
         VALUES ($1, $2, $3, $4, $5, 'active') RETURNING id",
    )
    .bind(username)
    .bind(format!("{username}@example.com"))
    .bind(&password_hash)
    .bind(username)
    .bind(role)
    .fetch_one(&app.state.database)
    .await
    .context("insert test user")?;
    Ok(id)
}

/// 走真实 login 端点换 Session Cookie（同时覆盖登录限流路径）。
async fn login_cookie(app: &TestApp, login: &str, password: &str) -> anyhow::Result<String> {
    let response = app
        .admin_post(
            "/api/admin/auth/login",
            serde_json::json!({ "login": login, "password": password }),
            None,
        )
        .await?;
    ensure!(response.status == StatusCode::OK, "{}", response.body);
    response
        .set_cookie
        .context("login did not set cookie")?
        .split(';')
        .next()
        .map(str::to_owned)
        .context("failed to parse session cookie")
}

/// 造一条 pending 访客评论（Public 提交端点属于第二批，这里直接入库作为前置数据）。
async fn insert_pending_comment(app: &TestApp, target_id: i64) -> anyhow::Result<i64> {
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO comments \
         (target_type, target_id, root_id, author_name, author_email, \
          content_markdown, content_html, status, ip_hash, user_agent_digest) \
         VALUES ('article', $1, NULL, 'Reader', 'reader@example.com', \
                 'A pending comment', '', 'pending', 'ip-hash', 'ua-digest') \
         RETURNING id",
    )
    .bind(target_id)
    .fetch_one(&app.state.database)
    .await
    .context("insert pending comment")?;
    Ok(id)
}

/// 造一篇文章作为评论目标，返回文章 id（标题需在用例内唯一，避免 Slug 冲突）。
async fn create_article(app: &TestApp, cookie: &str, title: &str) -> anyhow::Result<i64> {
    let created = app
        .admin_post(
            "/api/admin/articles",
            serde_json::json!({ "title": title, "markdown_source": "# c" }),
            Some(cookie),
        )
        .await?;
    ensure!(created.status == StatusCode::CREATED, "{}", created.body);
    created.body["id"].as_i64().context("missing article id")
}

/// 造一条指定状态/作者/内容的访客评论（直接入库作为前置数据）。
/// `age_secs` 把 created_at 向过去偏移，保证按 created_at DESC 排序的断言确定性。
async fn insert_comment(
    app: &TestApp,
    target_id: i64,
    status: &str,
    author_name: &str,
    content: &str,
    age_secs: i64,
) -> anyhow::Result<i64> {
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO comments \
         (target_type, target_id, root_id, author_name, author_email, \
          content_markdown, content_html, status, ip_hash, user_agent_digest, \
          created_at, updated_at) \
         VALUES ('article', $1, NULL, $2, 'reader@example.com', \
                 $3, '', $4, 'ip-hash', 'ua-digest', \
                 now() - make_interval(secs => $5), \
                 now() - make_interval(secs => $5)) \
         RETURNING id",
    )
    .bind(target_id)
    .bind(author_name)
    .bind(content)
    .bind(status)
    .bind(age_secs as f64)
    .fetch_one(&app.state.database)
    .await
    .context("insert comment")?;
    Ok(id)
}

/// 提交评论状态变更请求，返回完整响应（便于断言状态码与错误码）。
async fn change_status(
    app: &TestApp,
    cookie: &str,
    comment_id: i64,
    status: &str,
) -> anyhow::Result<common::TestResponse> {
    app.admin_patch(
        &format!("/api/admin/comments/{comment_id}/status"),
        serde_json::json!({ "status": status }),
        cookie,
    )
    .await
}

#[tokio::test]
async fn comment_admin_flow_covers_moderation_reply_delete_and_audit() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;
        let article_id = create_article(&app, &owner_cookie, "Commented Post").await?;
        let comment_id = insert_pending_comment(&app, article_id).await?;

        // 列表 + status 过滤：Admin 端可见访客 Email（审核判断依据），Public API 永不返回。
        let list = app
            .admin_get(
                "/api/admin/comments?status=pending&page_size=10",
                &owner_cookie,
            )
            .await?;
        ensure!(list.status == StatusCode::OK, "{}", list.body);
        ensure!(list.body["total"].as_i64() == Some(1));
        let item = &list.body["items"][0];
        ensure!(item["status"] == "pending");
        ensure!(item["id"].as_i64() == Some(comment_id));
        ensure!(item["author_email"].as_str() == Some("reader@example.com"));
        ensure!(item["is_admin_reply"] == false);

        // 非法状态转换 pending → recycled：core 状态机规则由 HTTP 层映射为 409。
        let bad_transition = app
            .admin_patch(
                &format!("/api/admin/comments/{comment_id}/status"),
                serde_json::json!({ "status": "recycled" }),
                &owner_cookie,
            )
            .await?;
        ensure!(
            bad_transition.status == StatusCode::CONFLICT,
            "{}",
            bad_transition.body
        );
        ensure!(bad_transition.body["error"]["code"] == "INVALID_COMMENT_TRANSITION");

        // 审核通过（带 reason 会持久化）。
        let approved = app
            .admin_patch(
                &format!("/api/admin/comments/{comment_id}/status"),
                serde_json::json!({ "status": "approved", "reason": "looks fine" }),
                &owner_cookie,
            )
            .await?;
        ensure!(approved.status == StatusCode::OK, "{}", approved.body);
        ensure!(approved.body["status"] == "approved");
        ensure!(approved.body["moderation_reason"].as_str() == Some("looks fine"));

        // 管理员回复：Markdown 渲染 + Sanitize（script 必须被清除）。
        let reply = app
            .admin_post(
                &format!("/api/admin/comments/{comment_id}/reply"),
                serde_json::json!({ "content_markdown": "Thanks! <script>alert(1)</script>" }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(reply.status == StatusCode::CREATED, "{}", reply.body);
        let reply_body = &reply.body;
        ensure!(reply_body["is_admin_reply"] == true);
        ensure!(reply_body["status"] == "approved");
        ensure!(reply_body["parent_id"].as_i64() == Some(comment_id));
        ensure!(reply_body["author_name"].as_str() == Some("Owner"));
        let reply_html = reply_body["content_html"]
            .as_str()
            .context("reply missing rendered html")?;
        ensure!(reply_html.contains("Thanks!"), "html: {reply_html}");
        ensure!(!reply_html.contains("<script>"), "html: {reply_html}");

        // 空内容回复 → 400。
        let empty_reply = app
            .admin_post(
                &format!("/api/admin/comments/{comment_id}/reply"),
                serde_json::json!({ "content_markdown": "   " }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(
            empty_reply.status == StatusCode::BAD_REQUEST,
            "{}",
            empty_reply.body
        );
        ensure!(empty_reply.body["error"]["code"] == "INVALID_COMMENT_CONTENT");

        // 未回收不允许物理删除（与文章删除语义一致）。
        let early_delete = app
            .admin_delete(&format!("/api/admin/comments/{comment_id}"), &owner_cookie)
            .await?;
        ensure!(
            early_delete.status == StatusCode::CONFLICT,
            "{}",
            early_delete.body
        );
        ensure!(early_delete.body["error"]["code"] == "COMMENT_NOT_RECYCLED");

        // approved → recycled → delete → 404。
        let recycled = app
            .admin_patch(
                &format!("/api/admin/comments/{comment_id}/status"),
                serde_json::json!({ "status": "recycled" }),
                &owner_cookie,
            )
            .await?;
        ensure!(recycled.status == StatusCode::OK, "{}", recycled.body);
        let deleted = app
            .admin_delete(&format!("/api/admin/comments/{comment_id}"), &owner_cookie)
            .await?;
        ensure!(deleted.status == StatusCode::NO_CONTENT, "{}", deleted.body);
        let gone = app
            .admin_get(&format!("/api/admin/comments/{comment_id}"), &owner_cookie)
            .await?;
        ensure!(gone.status == StatusCode::NOT_FOUND);

        // 审计落库：moderated ×2（approved/recycled）+ reply_created + deleted。
        let (audit_count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM audit_logs \
             WHERE target_type = 'comment' AND actor_user_id = 1 \
             AND action IN ('comment.moderated', 'comment.reply_created', 'comment.deleted')",
        )
        .fetch_one(&app.state.database)
        .await?;
        ensure!(
            audit_count >= 4,
            "expected >= 4 comment audit events, got {audit_count}"
        );

        // Dashboard：原评论已软删，只剩 1 条回复；pending 归零。
        let dashboard = app.admin_get("/api/admin/dashboard", &owner_cookie).await?;
        ensure!(dashboard.status == StatusCode::OK, "{}", dashboard.body);
        ensure!(dashboard.body["articles"]["total"].as_i64() == Some(1));
        ensure!(dashboard.body["articles"]["draft"].as_i64() == Some(1));
        ensure!(dashboard.body["comments"]["total"].as_i64() == Some(1));
        ensure!(dashboard.body["comments"]["pending"].as_i64() == Some(0));
        ensure!(dashboard.body["comments"]["today"].as_i64() == Some(1));
        ensure!(
            dashboard.body["recent_pending_comments"]
                .as_array()
                .map(Vec::len)
                .unwrap_or_default()
                == 0
        );
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn comment_dashboard_and_audit_permission_matrix() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let _owner_cookie = app.bootstrap_owner().await?;
        create_user(&app, "editor-a", "editor", "editor-pass-1").await?;
        create_user(&app, "moderator-a", "moderator", "moderator-pass-1").await?;
        let editor_cookie = login_cookie(&app, "editor-a", "editor-pass-1").await?;
        let moderator_cookie = login_cookie(&app, "moderator-a", "moderator-pass-1").await?;

        // editor：无 ModerateComments / ManageSettings → 评论与 Audit 均 403，Dashboard 可见。
        let denied = app.admin_get("/api/admin/comments", &editor_cookie).await?;
        ensure!(denied.status == StatusCode::FORBIDDEN, "{}", denied.body);
        ensure!(denied.body["error"]["code"] == "PERMISSION_DENIED");

        let denied_audit = app
            .admin_get("/api/admin/audit-logs", &editor_cookie)
            .await?;
        ensure!(denied_audit.status == StatusCode::FORBIDDEN);

        let editor_dashboard = app
            .admin_get("/api/admin/dashboard", &editor_cookie)
            .await?;
        ensure!(
            editor_dashboard.status == StatusCode::OK,
            "{}",
            editor_dashboard.body
        );

        // moderator：可访问评论，但 Audit 仅 Owner 可查。
        let allowed = app
            .admin_get("/api/admin/comments", &moderator_cookie)
            .await?;
        ensure!(allowed.status == StatusCode::OK, "{}", allowed.body);

        let denied_audit_mod = app
            .admin_get("/api/admin/audit-logs", &moderator_cookie)
            .await?;
        ensure!(denied_audit_mod.status == StatusCode::FORBIDDEN);

        // 未认证：401。
        let unauthenticated = app.get("/api/admin/comments").await?;
        ensure!(unauthenticated.status == StatusCode::UNAUTHORIZED);
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn audit_logs_support_filtering_and_pagination() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        // bootstrap 本身写一条 `auth.bootstrap` 审计，作为已知数据。
        let owner_cookie = app.bootstrap_owner().await?;

        let filtered = app
            .admin_get("/api/admin/audit-logs?action=auth.bootstrap", &owner_cookie)
            .await?;
        ensure!(filtered.status == StatusCode::OK, "{}", filtered.body);
        ensure!(filtered.body["total"].as_i64() == Some(1));
        let item = &filtered.body["items"][0];
        ensure!(item["action"] == "auth.bootstrap");
        ensure!(item["actor_username"].as_str() == Some("owner"));
        ensure!(item["target_type"] == "user");

        // 分页：page_size 生效，total 语义为过滤后的总数。
        let paged = app
            .admin_get("/api/admin/audit-logs?page=1&page_size=1", &owner_cookie)
            .await?;
        ensure!(paged.status == StatusCode::OK, "{}", paged.body);
        ensure!(paged.body["page_size"].as_u64() == Some(1));
        ensure!(
            paged.body["items"].as_array().map(Vec::len) == Some(1),
            "{}",
            paged.body
        );
        ensure!(paged.body["total"].as_i64() == Some(1));

        // 时间范围：end 指向过去 → 0 条（created_at 左闭右开）。
        let past = app
            .admin_get(
                "/api/admin/audit-logs?end=2020-01-01T00:00:00Z",
                &owner_cookie,
            )
            .await?;
        ensure!(past.status == StatusCode::OK, "{}", past.body);
        ensure!(past.body["total"].as_i64() == Some(0));

        // 非法筛选值不应 500：未知 action 返回 0 条。
        let unknown = app
            .admin_get("/api/admin/audit-logs?action=does-not-exist", &owner_cookie)
            .await?;
        ensure!(unknown.status == StatusCode::OK, "{}", unknown.body);
        ensure!(unknown.body["total"].as_i64() == Some(0));
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn comment_list_filters_pagination_and_query_validation() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;
        let article_a = create_article(&app, &owner_cookie, "Filter Post A").await?;
        let article_b = create_article(&app, &owner_cookie, "Filter Post B").await?;

        // 四条评论：不同状态/作者/内容/目标，created_at 依次拉开（c4 最新）。
        let c1 =
            insert_comment(&app, article_a, "pending", "Alice", "hello world first", 30).await?;
        let c2 = insert_comment(&app, article_a, "approved", "Bob", "second note", 20).await?;
        let c3 = insert_comment(&app, article_a, "spam", "Carol", "buy cheap stuff", 10).await?;
        let c4 = insert_comment(
            &app,
            article_b,
            "pending",
            "Dave",
            "other post 100% sure_ok",
            0,
        )
        .await?;

        // 默认分页参数：page=1、page_size=20，按 created_at DESC 排序（最新在前）。
        let all = app.admin_get("/api/admin/comments", &owner_cookie).await?;
        ensure!(all.status == StatusCode::OK, "{}", all.body);
        ensure!(all.body["total"].as_i64() == Some(4));
        ensure!(all.body["page"].as_u64() == Some(1));
        ensure!(all.body["page_size"].as_u64() == Some(20));
        ensure!(all.body["items"].as_array().map(Vec::len) == Some(4));
        ensure!(
            all.body["items"][0]["id"].as_i64() == Some(c4),
            "{}",
            all.body
        );

        // status / target_id / target_type 过滤。
        let approved = app
            .admin_get("/api/admin/comments?status=approved", &owner_cookie)
            .await?;
        ensure!(approved.body["total"].as_i64() == Some(1));
        ensure!(approved.body["items"][0]["id"].as_i64() == Some(c2));

        let by_target = app
            .admin_get(
                &format!("/api/admin/comments?target_id={article_b}"),
                &owner_cookie,
            )
            .await?;
        ensure!(by_target.body["total"].as_i64() == Some(1));
        ensure!(by_target.body["items"][0]["id"].as_i64() == Some(c4));

        let by_type = app
            .admin_get("/api/admin/comments?target_type=page", &owner_cookie)
            .await?;
        ensure!(by_type.status == StatusCode::OK, "{}", by_type.body);
        ensure!(by_type.body["total"].as_i64() == Some(0));

        // keyword：同时命中内容（ILIKE）与作者昵称；纯空白关键字视为未传。
        let by_content = app
            .admin_get("/api/admin/comments?keyword=cheap", &owner_cookie)
            .await?;
        ensure!(by_content.body["total"].as_i64() == Some(1));
        ensure!(by_content.body["items"][0]["id"].as_i64() == Some(c3));

        let by_author = app
            .admin_get("/api/admin/comments?keyword=alice", &owner_cookie)
            .await?;
        ensure!(by_author.body["total"].as_i64() == Some(1));
        ensure!(by_author.body["items"][0]["id"].as_i64() == Some(c1));

        let blank_keyword = app
            .admin_get("/api/admin/comments?keyword=%20%20", &owner_cookie)
            .await?;
        ensure!(blank_keyword.body["total"].as_i64() == Some(4));

        // 特殊字符走参数化绑定，不得 500：`%`/`_` 按字面量匹配
        // （ILIKE ESCAPE '\'），不再作为通配符匹配全部。
        let quoted = app
            .admin_get("/api/admin/comments?keyword=it%27s", &owner_cookie)
            .await?;
        ensure!(quoted.status == StatusCode::OK, "{}", quoted.body);
        ensure!(quoted.body["total"].as_i64() == Some(0));
        // `%` 只命中内容中确实含字面量 % 的 c4。
        let percent = app
            .admin_get("/api/admin/comments?keyword=%25", &owner_cookie)
            .await?;
        ensure!(percent.body["total"].as_i64() == Some(1));
        ensure!(percent.body["items"][0]["id"].as_i64() == Some(c4));
        // `_` 只命中内容中确实含字面量 _ 的 c4；含 _ 的多字符关键字照常子串匹配。
        let underscore = app
            .admin_get("/api/admin/comments?keyword=%5F", &owner_cookie)
            .await?;
        ensure!(underscore.body["total"].as_i64() == Some(1));
        ensure!(underscore.body["items"][0]["id"].as_i64() == Some(c4));
        let phrase = app
            .admin_get("/api/admin/comments?keyword=sure_ok", &owner_cookie)
            .await?;
        ensure!(phrase.body["total"].as_i64() == Some(1));
        ensure!(phrase.body["items"][0]["id"].as_i64() == Some(c4));

        // 分页：page_size=2 时前两页各 2 条且按序不重复，第三页为空，total 恒为过滤后总数。
        let page1 = app
            .admin_get("/api/admin/comments?page=1&page_size=2", &owner_cookie)
            .await?;
        let page2 = app
            .admin_get("/api/admin/comments?page=2&page_size=2", &owner_cookie)
            .await?;
        let page3 = app
            .admin_get("/api/admin/comments?page=3&page_size=2", &owner_cookie)
            .await?;
        ensure!(page1.body["total"].as_i64() == Some(4));
        ensure!(page2.body["total"].as_i64() == Some(4));
        ensure!(page3.body["items"].as_array().map(Vec::len) == Some(0));
        let ids_of = |body: &serde_json::Value| {
            body["items"]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item["id"].as_i64())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        };
        ensure!(ids_of(&page1.body) == vec![c4, c3], "{}", page1.body);
        ensure!(ids_of(&page2.body) == vec![c2, c1], "{}", page2.body);

        // page=0 归一化为 1；page_size 超上限收敛到 100。
        let page_zero = app
            .admin_get("/api/admin/comments?page=0&page_size=2", &owner_cookie)
            .await?;
        ensure!(page_zero.status == StatusCode::OK, "{}", page_zero.body);
        ensure!(page_zero.body["page"].as_u64() == Some(1));
        let clamped = app
            .admin_get("/api/admin/comments?page_size=500", &owner_cookie)
            .await?;
        ensure!(clamped.body["page_size"].as_u64() == Some(100));

        // 非法筛选值 → 400（不静默忽略，也不得 500）。
        let bad_status = app
            .admin_get("/api/admin/comments?status=bogus", &owner_cookie)
            .await?;
        ensure!(bad_status.status == StatusCode::BAD_REQUEST);
        ensure!(bad_status.body["error"]["code"] == "INVALID_COMMENT_STATUS");
        let bad_type = app
            .admin_get("/api/admin/comments?target_type=bogus", &owner_cookie)
            .await?;
        ensure!(bad_type.status == StatusCode::BAD_REQUEST);
        ensure!(bad_type.body["error"]["code"] == "INVALID_COMMENT_TARGET");
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn comment_status_transition_matrix_is_enforced() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;
        let article_id = create_article(&app, &owner_cookie, "Transition Post").await?;

        // 合法链路一：pending → approved → recycled → approved（回收站恢复）。
        let chain = insert_comment(&app, article_id, "pending", "T1", "chain one", 0).await?;
        let r = change_status(&app, &owner_cookie, chain, "approved").await?;
        ensure!(r.status == StatusCode::OK, "{}", r.body);
        let r = change_status(&app, &owner_cookie, chain, "recycled").await?;
        ensure!(r.status == StatusCode::OK, "{}", r.body);
        let r = change_status(&app, &owner_cookie, chain, "approved").await?;
        ensure!(r.status == StatusCode::OK, "{}", r.body);
        // 不传 reason 时 moderation_reason 清空。
        ensure!(r.body["moderation_reason"].is_null(), "{}", r.body);

        // 合法链路二：pending → rejected → recycled。
        let rejected_chain =
            insert_comment(&app, article_id, "pending", "T2", "chain two", 0).await?;
        let r = change_status(&app, &owner_cookie, rejected_chain, "rejected").await?;
        ensure!(r.status == StatusCode::OK, "{}", r.body);
        let r = change_status(&app, &owner_cookie, rejected_chain, "recycled").await?;
        ensure!(r.status == StatusCode::OK, "{}", r.body);

        // 合法链路三：pending → spam → recycled。
        let spam_chain =
            insert_comment(&app, article_id, "pending", "T3", "chain three", 0).await?;
        let r = change_status(&app, &owner_cookie, spam_chain, "spam").await?;
        ensure!(r.status == StatusCode::OK, "{}", r.body);
        let r = change_status(&app, &owner_cookie, spam_chain, "recycled").await?;
        ensure!(r.status == StatusCode::OK, "{}", r.body);

        // 非法迁移矩阵：与 core 状态机（can_transition_to）一一对应，全部应映射为 409。
        let cases: [(&str, &[&str]); 5] = [
            ("pending", &["pending", "recycled"]),
            ("approved", &["pending", "approved", "rejected", "spam"]),
            ("rejected", &["pending", "approved", "rejected", "spam"]),
            ("spam", &["pending", "approved", "rejected", "spam"]),
            ("recycled", &["pending", "rejected", "spam", "recycled"]),
        ];
        for (index, (from, targets)) in cases.iter().enumerate() {
            let id = insert_comment(
                &app,
                article_id,
                from,
                "Matrix",
                &format!("matrix case {from}"),
                index as i64,
            )
            .await?;
            for target in *targets {
                let r = change_status(&app, &owner_cookie, id, target).await?;
                ensure!(
                    r.status == StatusCode::CONFLICT,
                    "{from} -> {target}: {}",
                    r.body
                );
                ensure!(r.body["error"]["code"] == "INVALID_COMMENT_TRANSITION");
            }
            // 非法迁移不应改变原状态。
            let detail = app
                .admin_get(&format!("/api/admin/comments/{id}"), &owner_cookie)
                .await?;
            ensure!(
                detail.body["status"].as_str() == Some(*from),
                "{from} mutated"
            );
        }

        // 不存在的评论 → 404。
        let missing = change_status(&app, &owner_cookie, 999_999, "approved").await?;
        ensure!(missing.status == StatusCode::NOT_FOUND);
        ensure!(missing.body["error"]["code"] == "COMMENT_NOT_FOUND");

        // 非法状态值 → 400。
        let bad_value = change_status(&app, &owner_cookie, chain, "bogus").await?;
        ensure!(bad_value.status == StatusCode::BAD_REQUEST);
        ensure!(bad_value.body["error"]["code"] == "INVALID_COMMENT_STATUS");
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn comment_reply_threading_and_content_boundaries() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;
        let article_id = create_article(&app, &owner_cookie, "Reply Post").await?;
        let comment_id = insert_comment(&app, article_id, "approved", "Reader", "root", 0).await?;

        // 回复不存在的评论 → 404。
        let missing = app
            .admin_post(
                "/api/admin/comments/999999/reply",
                serde_json::json!({ "content_markdown": "hi" }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(missing.status == StatusCode::NOT_FOUND);
        ensure!(missing.body["error"]["code"] == "COMMENT_NOT_FOUND");

        // 内容长度边界：2001 字符 → 400；恰好 2000 字符 → 201（上限含边界）。
        let too_long = app
            .admin_post(
                &format!("/api/admin/comments/{comment_id}/reply"),
                serde_json::json!({ "content_markdown": "a".repeat(2001) }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(too_long.status == StatusCode::BAD_REQUEST);
        ensure!(too_long.body["error"]["code"] == "INVALID_COMMENT_CONTENT");

        let at_max = app
            .admin_post(
                &format!("/api/admin/comments/{comment_id}/reply"),
                serde_json::json!({ "content_markdown": "a".repeat(2000) }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(at_max.status == StatusCode::CREATED, "{}", at_max.body);

        // 线程关系：回复的 parent 指向原评论，root 指向线程根
        // （原评论直接入库时 root_id 为 NULL，Repository 回填取父评论自身 id）。
        ensure!(at_max.body["parent_id"].as_i64() == Some(comment_id));
        ensure!(at_max.body["root_id"].as_i64() == Some(comment_id));
        ensure!(at_max.body["target_id"].as_i64() == Some(article_id));
        ensure!(at_max.body["is_admin_reply"] == true);

        // 二级回复：parent 指向直接父回复，root 保持线程根不变。
        let first_reply_id = at_max.body["id"].as_i64().context("missing reply id")?;
        let nested = app
            .admin_post(
                &format!("/api/admin/comments/{first_reply_id}/reply"),
                serde_json::json!({ "content_markdown": "nested reply" }),
                Some(&owner_cookie),
            )
            .await?;
        ensure!(nested.status == StatusCode::CREATED, "{}", nested.body);
        ensure!(nested.body["parent_id"].as_i64() == Some(first_reply_id));
        ensure!(nested.body["root_id"].as_i64() == Some(comment_id));

        // 按 target 过滤的列表包含原评论 + 两条回复。
        let list = app
            .admin_get(
                &format!("/api/admin/comments?target_id={article_id}"),
                &owner_cookie,
            )
            .await?;
        ensure!(list.body["total"].as_i64() == Some(3), "{}", list.body);
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn comment_get_and_delete_error_paths() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;
        let article_id = create_article(&app, &owner_cookie, "Delete Post").await?;

        // 详情 / 删除不存在的评论 → 404。
        let missing = app
            .admin_get("/api/admin/comments/999999", &owner_cookie)
            .await?;
        ensure!(missing.status == StatusCode::NOT_FOUND);
        ensure!(missing.body["error"]["code"] == "COMMENT_NOT_FOUND");
        let delete_missing = app
            .admin_delete("/api/admin/comments/999999", &owner_cookie)
            .await?;
        ensure!(delete_missing.status == StatusCode::NOT_FOUND);
        ensure!(delete_missing.body["error"]["code"] == "COMMENT_NOT_FOUND");

        // pending / spam 未进入回收站不可物理删除 → 409。
        let pending_id = insert_comment(&app, article_id, "pending", "P", "pending one", 0).await?;
        let early = app
            .admin_delete(&format!("/api/admin/comments/{pending_id}"), &owner_cookie)
            .await?;
        ensure!(early.status == StatusCode::CONFLICT, "{}", early.body);
        ensure!(early.body["error"]["code"] == "COMMENT_NOT_RECYCLED");

        let spam_id = insert_comment(&app, article_id, "spam", "S", "spam one", 0).await?;
        let early_spam = app
            .admin_delete(&format!("/api/admin/comments/{spam_id}"), &owner_cookie)
            .await?;
        ensure!(
            early_spam.status == StatusCode::CONFLICT,
            "{}",
            early_spam.body
        );

        // spam → recycled → delete → 204；删除后详情 404、列表不再返回；重复删除 404。
        let recycled = change_status(&app, &owner_cookie, spam_id, "recycled").await?;
        ensure!(recycled.status == StatusCode::OK, "{}", recycled.body);
        let deleted = app
            .admin_delete(&format!("/api/admin/comments/{spam_id}"), &owner_cookie)
            .await?;
        ensure!(deleted.status == StatusCode::NO_CONTENT, "{}", deleted.body);
        let gone = app
            .admin_get(&format!("/api/admin/comments/{spam_id}"), &owner_cookie)
            .await?;
        ensure!(gone.status == StatusCode::NOT_FOUND);
        let list = app
            .admin_get(
                &format!("/api/admin/comments?target_id={article_id}"),
                &owner_cookie,
            )
            .await?;
        // 只剩 pending 那一条（spam 已软删）。
        ensure!(list.body["total"].as_i64() == Some(1), "{}", list.body);
        let again = app
            .admin_delete(&format!("/api/admin/comments/{spam_id}"), &owner_cookie)
            .await?;
        ensure!(again.status == StatusCode::NOT_FOUND);
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}

#[tokio::test]
async fn comment_endpoints_enforce_authentication_and_moderation_permission() -> anyhow::Result<()>
{
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let owner_cookie = app.bootstrap_owner().await?;
        create_user(&app, "editor-b", "editor", "editor-pass-2").await?;
        create_user(&app, "moderator-b", "moderator", "moderator-pass-2").await?;
        let editor_cookie = login_cookie(&app, "editor-b", "editor-pass-2").await?;
        let moderator_cookie = login_cookie(&app, "moderator-b", "moderator-pass-2").await?;
        let article_id = create_article(&app, &owner_cookie, "Guard Post").await?;
        let comment_id = insert_comment(&app, article_id, "pending", "Reader", "target", 0).await?;

        // 未认证（无 Session 或伪造 Session）→ 401，详情/审核/回复/删除四个端点全覆盖。
        let bogus = "aries_admin_session=forged-token-value";
        let r = app
            .get(&format!("/api/admin/comments/{comment_id}"))
            .await?;
        ensure!(r.status == StatusCode::UNAUTHORIZED);
        let r = app
            .admin_patch(
                &format!("/api/admin/comments/{comment_id}/status"),
                serde_json::json!({ "status": "approved" }),
                bogus,
            )
            .await?;
        ensure!(r.status == StatusCode::UNAUTHORIZED);
        let r = app
            .admin_post(
                &format!("/api/admin/comments/{comment_id}/reply"),
                serde_json::json!({ "content_markdown": "hi" }),
                None,
            )
            .await?;
        ensure!(r.status == StatusCode::UNAUTHORIZED);
        let r = app
            .admin_delete(&format!("/api/admin/comments/{comment_id}"), bogus)
            .await?;
        ensure!(r.status == StatusCode::UNAUTHORIZED);

        // editor 无 ModerateComments 权限 → 403（列表 403 已由权限矩阵用例覆盖）。
        let r = app
            .admin_get(&format!("/api/admin/comments/{comment_id}"), &editor_cookie)
            .await?;
        ensure!(r.status == StatusCode::FORBIDDEN);
        ensure!(r.body["error"]["code"] == "PERMISSION_DENIED");
        let r = change_status(&app, &editor_cookie, comment_id, "approved").await?;
        ensure!(r.status == StatusCode::FORBIDDEN);
        let r = app
            .admin_post(
                &format!("/api/admin/comments/{comment_id}/reply"),
                serde_json::json!({ "content_markdown": "editor reply" }),
                Some(&editor_cookie),
            )
            .await?;
        ensure!(r.status == StatusCode::FORBIDDEN);
        let r = app
            .admin_delete(&format!("/api/admin/comments/{comment_id}"), &editor_cookie)
            .await?;
        ensure!(r.status == StatusCode::FORBIDDEN);

        // 被拒请求不应产生副作用：评论仍为 pending。
        let detail = app
            .admin_get(&format!("/api/admin/comments/{comment_id}"), &owner_cookie)
            .await?;
        ensure!(detail.body["status"] == "pending");

        // moderator 拥有 ModerateComments：可审核、回复（署名取 Display Name）、回收并删除。
        let r = change_status(&app, &moderator_cookie, comment_id, "spam").await?;
        ensure!(r.status == StatusCode::OK, "{}", r.body);
        let r = app
            .admin_post(
                &format!("/api/admin/comments/{comment_id}/reply"),
                serde_json::json!({ "content_markdown": "moderator reply" }),
                Some(&moderator_cookie),
            )
            .await?;
        ensure!(r.status == StatusCode::CREATED, "{}", r.body);
        ensure!(r.body["author_name"].as_str() == Some("moderator-b"));
        let r = change_status(&app, &moderator_cookie, comment_id, "recycled").await?;
        ensure!(r.status == StatusCode::OK, "{}", r.body);
        let r = app
            .admin_delete(
                &format!("/api/admin/comments/{comment_id}"),
                &moderator_cookie,
            )
            .await?;
        ensure!(r.status == StatusCode::NO_CONTENT, "{}", r.body);
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}
