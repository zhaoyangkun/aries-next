//! 主流程 E2E：创建 → 发布 → Public 可见 → 回收 → 不可见 → 恢复 → 再发布可见。

mod common;

use anyhow::{Context, ensure};
use axum::http::StatusCode;

use common::TestApp;

async fn transition(
    app: &TestApp,
    cookie: &str,
    id: i64,
    command: &str,
    expected_version: i64,
) -> anyhow::Result<serde_json::Value> {
    let response = app
        .admin_patch(
            &format!("/api/admin/articles/{id}/status"),
            serde_json::json!({ "command": command, "expected_version": expected_version }),
            cookie,
        )
        .await?;
    ensure!(
        response.status == StatusCode::OK,
        "{command} failed: {}",
        response.body
    );
    Ok(response.body)
}

async fn public_total(app: &TestApp) -> anyhow::Result<i64> {
    let list = app.get("/api/public/articles").await?;
    ensure!(list.status == StatusCode::OK, "{}", list.body);
    list.body["total"]
        .as_i64()
        .context("list total is not an integer")
}

async fn public_detail_status(app: &TestApp, slug: &str) -> anyhow::Result<StatusCode> {
    Ok(app
        .get(&format!("/api/public/articles/{slug}"))
        .await?
        .status)
}

#[tokio::test]
async fn article_lifecycle_matches_public_visibility() -> anyhow::Result<()> {
    let Some(app) = common::maybe_app().await? else {
        return Ok(());
    };
    let scenario = async {
        let cookie = app.bootstrap_owner().await?;

        // 1. 创建 Draft。
        let created = app
            .admin_post(
                "/api/admin/articles",
                serde_json::json!({
                    "title": "Lifecycle Post",
                    "markdown_source": "# Lifecycle",
                }),
                Some(&cookie),
            )
            .await?;
        ensure!(created.status == StatusCode::CREATED, "{}", created.body);
        let article = created.body;
        let id = article["id"].as_i64().context("missing id")?;
        let version = article["version"].as_i64().context("missing version")?;
        let slug = article["slug"].as_str().context("missing slug")?.to_owned();

        // 2. Draft 阶段 Public 不可见。
        ensure!(public_total(&app).await? == 0);
        ensure!(public_detail_status(&app, &slug).await? == StatusCode::NOT_FOUND);

        // 3. 发布后 Public 可见。
        let published = transition(&app, &cookie, id, "publish", version).await?;
        ensure!(published["status"] == "published");
        ensure!(public_total(&app).await? == 1);
        ensure!(public_detail_status(&app, &slug).await? == StatusCode::OK);
        let published_version = published["version"].as_i64().context("missing version")?;

        // 4. 回收后 Public 不可见。
        let recycled = transition(&app, &cookie, id, "recycle", published_version).await?;
        ensure!(recycled["status"] == "recycled");
        ensure!(public_total(&app).await? == 0);
        let gone = app.get(&format!("/api/public/articles/{slug}")).await?;
        ensure!(gone.status == StatusCode::NOT_FOUND);
        ensure!(gone.body["error"]["code"] == "ARTICLE_NOT_FOUND");
        let recycled_version = recycled["version"].as_i64().context("missing version")?;

        // 5. 恢复回到 Draft，Public 仍不可见；再次发布后恢复可见。
        let recovered = transition(&app, &cookie, id, "recover", recycled_version).await?;
        ensure!(recovered["status"] == "draft");
        ensure!(public_total(&app).await? == 0);
        let recovered_version = recovered["version"].as_i64().context("missing version")?;
        transition(&app, &cookie, id, "publish", recovered_version).await?;
        ensure!(public_total(&app).await? == 1);
        ensure!(public_detail_status(&app, &slug).await? == StatusCode::OK);
        Ok(())
    }
    .await;

    let cleanup_result = app.cleanup().await;
    scenario?;
    cleanup_result
}
