//! `sys_settings` + `sys_setting_items` → `site_settings` 单行 + `setting_groups`
//! （appearance / email / integrations）。显式 Key 白名单映射；Secret（如 smtp `pwd`）
//! 只写入不回读、不进 Report/Archive；未识别 Key 进 Archive（疑似 Secret 的 Key 只记名不记值）。

use std::collections::{BTreeMap, HashMap};

use anyhow::Context;
use serde_json::{Map, Value};
use sqlx::{FromRow, MySqlPool};

use super::{ArchiveEntry, TransformOutcome, apply_outcome};
use crate::context::MigrateCtx;

pub const TABLE: &str = "settings";

/// 未识别且疑似 Secret 的 Key：值不落盘、不进 Archive/Report，只记 Key 名。
pub fn is_secret_key(key: &str) -> bool {
    matches!(
        key,
        "pwd" | "password" | "token" | "secret_id" | "secret_key" | "app_secret" | "access_token"
    )
}

#[derive(Debug, Clone, FromRow)]
pub struct LegacySettingGroup {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone, FromRow)]
pub struct LegacySettingItem {
    pub id: i64,
    pub sys_id: i64,
    pub key: String,
    pub val: String,
}

#[derive(Debug, Default)]
pub struct SettingsPlan {
    /// site_settings 文本列（列名为白名单常量）。
    pub site_text: BTreeMap<&'static str, String>,
    /// site_settings 整数列（page_size_* ）。
    pub site_int: BTreeMap<&'static str, i32>,
    pub appearance: Map<String, Value>,
    pub email: Map<String, Value>,
    pub integrations: Map<String, Value>,
    pub archives: Vec<ArchiveEntry>,
    /// 被跳过的 Secret Key 名（不含值）。
    pub skipped_secrets: Vec<String>,
    pub notes: Vec<String>,
    pub migrated: i64,
    /// 旧站评论开关（评论设置.is_on），finalize 时合成 comment_policy。
    pub comment_is_on: Option<bool>,
    /// 旧站评论审核开关（评论设置.is_review_on）。
    pub comment_is_review: Option<bool>,
}

impl SettingsPlan {
    /// 合成 comment_policy：关评论 → closed；开审核 → moderated；否则 auto_approve。
    /// 旧站无评论设置时保持目标默认值 moderated，不写入。
    /// comment_policy 是派生值，不计 migrated（源 Key is_on/is_review_on 已分别计数）。
    pub fn finalize(&mut self) {
        let policy = match (self.comment_is_on, self.comment_is_review) {
            (Some(false), _) => Some("closed"),
            (Some(true), Some(true)) | (Some(true), None) => Some("moderated"),
            (Some(true), Some(false)) => Some("auto_approve"),
            (None, _) => None,
        };
        if let Some(policy) = policy {
            self.site_text.insert("comment_policy", policy.to_owned());
        }
    }

    /// 整数型设置项：越界或解析失败进 Archive，不落库。
    fn set_page_size(
        &mut self,
        item_id: i64,
        group: &str,
        key: &str,
        val: &str,
        column: &'static str,
        range: std::ops::RangeInclusive<i32>,
    ) -> bool {
        match val.trim().parse::<i32>() {
            Ok(size) if range.contains(&size) => {
                self.set_site_int(column, size);
                true
            }
            _ => {
                self.archive_item(
                    item_id,
                    group,
                    key,
                    val,
                    &format!("value is not an integer in {range:?}"),
                );
                false
            }
        }
    }
}

/// 设置项映射结果（纯函数返回，计数在 migrate 调用点按条目累计）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MappingOutcome {
    /// 识别并已落入 plan。
    Applied,
    /// 识别但值非法，整条已归档。
    Archived,
    /// 未识别的 Key。
    Unrecognized,
}

/// 单条设置项的白名单映射（纯函数）。
pub fn map_setting_item(
    item_id: i64,
    group_name: &str,
    key: &str,
    val: &str,
    plan: &mut SettingsPlan,
) -> MappingOutcome {
    match (group_name, key) {
        ("网站设置", "site_name") => plan.set_site_text("site_name", val),
        ("网站设置", "site_desc") => plan.set_site_text("site_description", val),
        ("网站设置", "site_url") => plan.set_site_text("site_url", val),
        ("网站设置", "site_logo") => {
            plan.set_site_text("logo_url", val);
            plan.set_appearance("logo_url", Value::String(val.to_owned()));
        }
        ("网站设置", "record_number") => plan.set_site_text("icp_text", val),
        // 分页尺寸在旧库「参数设置」分组下。
        ("参数设置", "index_page_size") => {
            if !plan.set_page_size(
                item_id,
                group_name,
                key,
                val,
                "page_size_index",
                1..=i32::MAX,
            ) {
                return MappingOutcome::Archived;
            }
        }
        ("参数设置", "archive_page_size") => {
            if !plan.set_page_size(
                item_id,
                group_name,
                key,
                val,
                "page_size_archive",
                1..=i32::MAX,
            ) {
                return MappingOutcome::Archived;
            }
        }
        ("参数设置", "site_map_page_size") => {
            if !plan.set_page_size(
                item_id,
                group_name,
                key,
                val,
                "page_size_search",
                1..=i32::MAX,
            ) {
                return MappingOutcome::Archived;
            }
        }
        // 评论策略：is_on/is_review_on 先记原始布尔，finalize 时合成 comment_policy。
        ("评论设置", "is_on") => {
            plan.comment_is_on = Some(val.trim() == "1");
        }
        ("评论设置", "is_review_on") => {
            plan.comment_is_review = Some(val.trim() == "1");
        }
        ("评论设置", "page_size") => {
            // 目标 CHECK：comments_per_page BETWEEN 5 AND 100。
            if !plan.set_page_size(item_id, group_name, key, val, "comments_per_page", 5..=100) {
                return MappingOutcome::Archived;
            }
        }
        ("邮件设置", "address") => {
            // 空地址视为「已识别但不启用」；enabled 是派生标记，不计迁移数。
            if !val.trim().is_empty() {
                plan.email.insert("enabled".to_owned(), Value::Bool(true));
                plan.set_email("smtp_host", Value::String(val.to_owned()));
            }
        }
        ("邮件设置", "port") => match val.parse::<u16>() {
            Ok(port) => plan.set_email("smtp_port", Value::from(port)),
            Err(_) => {
                plan.archive_item(item_id, group_name, key, val, "smtp port is not a u16");
                return MappingOutcome::Archived;
            }
        },
        ("邮件设置", "account") => {
            plan.set_email("smtp_username", Value::String(val.to_owned()));
            plan.set_email("from_address", Value::String(val.to_owned()));
        }
        // Secret：只写入目标库，不回读、不进 Report/Archive。
        ("邮件设置", "pwd") => {
            if !val.is_empty() {
                plan.set_email("smtp_password", Value::String(val.to_owned()));
            }
        }
        ("邮件设置", "sender") => plan.set_email("from_name", Value::String(val.to_owned())),
        _ => return MappingOutcome::Unrecognized,
    }
    MappingOutcome::Applied
}

impl SettingsPlan {
    fn set_site_text(&mut self, column: &'static str, value: &str) {
        self.site_text.insert(column, value.to_owned());
    }

    fn set_site_int(&mut self, column: &'static str, value: i32) {
        self.site_int.insert(column, value);
    }

    fn set_appearance(&mut self, key: &str, value: Value) {
        self.appearance.insert(key.to_owned(), value);
    }

    fn set_email(&mut self, key: &str, value: Value) {
        self.email.insert(key.to_owned(), value);
    }

    fn archive_item(&mut self, id: i64, group: &str, key: &str, val: &str, reason: &str) {
        self.archives.push(ArchiveEntry {
            id: Some(id),
            fields: serde_json::json!({ "group": group, "key": key, "val": val }),
            reason: reason.to_owned(),
            whole_row: true,
        });
    }
}

async fn extract_groups(mysql: &MySqlPool) -> anyhow::Result<Vec<LegacySettingGroup>> {
    sqlx::query_as::<_, LegacySettingGroup>(
        "SELECT CAST(id AS SIGNED) AS id, name FROM sys_settings ORDER BY id",
    )
    .fetch_all(mysql)
    .await
    .context("failed to extract legacy sys_settings")
}

async fn extract_items(mysql: &MySqlPool) -> anyhow::Result<Vec<LegacySettingItem>> {
    sqlx::query_as::<_, LegacySettingItem>(
        "SELECT CAST(id AS SIGNED) AS id, CAST(sys_id AS SIGNED) AS sys_id, `key`, val \
         FROM sys_setting_items ORDER BY id",
    )
    .fetch_all(mysql)
    .await
    .context("failed to extract legacy sys_setting_items")
}

pub async fn migrate(ctx: &mut MigrateCtx) -> anyhow::Result<()> {
    let groups = extract_groups(&ctx.mysql).await?;
    let items = extract_items(&ctx.mysql).await?;
    ctx.report.table(TABLE).source_rows = items.len() as i64;

    let group_names: HashMap<i64, String> = groups
        .into_iter()
        .map(|group| (group.id, group.name))
        .collect();

    let mut plan = SettingsPlan::default();
    for item in &items {
        let group_name = group_names
            .get(&item.sys_id)
            .map(String::as_str)
            .unwrap_or("<unknown>");
        // 按条目计数：识别即 migrated（含派生多写）；整条归档由 archive_item 记录；
        // 未识别的 Secret Key 只记名（skipped），值不落盘。
        match map_setting_item(item.id, group_name, &item.key, &item.val, &mut plan) {
            MappingOutcome::Applied => plan.migrated += 1,
            MappingOutcome::Archived => {}
            MappingOutcome::Unrecognized => {
                if is_secret_key(&item.key) {
                    plan.skipped_secrets
                        .push(format!("{group_name}.{key}", key = item.key));
                } else {
                    plan.archive_item(
                        item.id,
                        group_name,
                        &item.key,
                        &item.val,
                        "unrecognized setting key",
                    );
                }
            }
        }
    }
    plan.finalize();

    let migrated = plan.migrated;
    let skipped_secrets = std::mem::take(&mut plan.skipped_secrets);
    let outcome: TransformOutcome<()> = TransformOutcome {
        row: None,
        archives: std::mem::take(&mut plan.archives),
        repaired: 0,
        notes: std::mem::take(&mut plan.notes),
    };
    apply_outcome(ctx, TABLE, outcome)?;

    // site_settings：单行表，UPDATE 预插行（id=1）。列名来自白名单常量。
    if !plan.site_text.is_empty() || !plan.site_int.is_empty() {
        let mut assignments = String::new();
        for (index, column) in (1usize..).zip(plan.site_text.keys().chain(plan.site_int.keys())) {
            if index > 1 {
                assignments.push_str(", ");
            }
            assignments.push_str(&format!("{column} = ${index}"));
        }
        let query =
            format!("UPDATE site_settings SET {assignments}, updated_at = now() WHERE id = 1");
        let mut built = sqlx::query(&query);
        for value in plan.site_text.values() {
            built = built.bind(value);
        }
        for value in plan.site_int.values() {
            built = built.bind(value);
        }
        built
            .execute(&ctx.pg)
            .await
            .context("failed to update site_settings")?;
    }

    // setting_groups：jsonb 合并（不重置 version，迁移不是乐观锁场景）。
    for (group, payload) in [
        ("appearance", &plan.appearance),
        ("email", &plan.email),
        ("integrations", &plan.integrations),
    ] {
        if payload.is_empty() {
            continue;
        }
        sqlx::query(
            "UPDATE setting_groups SET payload = payload || $1::jsonb, updated_at = now() \
             WHERE grp = $2",
        )
        .bind(Value::Object(payload.clone()))
        .bind(group)
        .execute(&ctx.pg)
        .await
        .with_context(|| format!("failed to update setting_groups.{group}"))?;
    }

    let report = ctx.report.table(TABLE);
    report.migrated = migrated;
    // 未识别的 Secret Key 计入 skipped（值不落盘）。
    report.skipped = skipped_secrets.len() as i64;
    if !skipped_secrets.is_empty() {
        report.note(format!(
            "secret keys applied or skipped without recording values: {}",
            skipped_secrets.join(", ")
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn site_keys_map_to_site_settings_columns() {
        let mut plan = SettingsPlan::default();
        assert!(matches!(
            map_setting_item(1, "网站设置", "site_name", "我的博客", &mut plan),
            MappingOutcome::Applied | MappingOutcome::Archived
        ));
        assert!(matches!(
            map_setting_item(2, "网站设置", "record_number", "京ICP备xxx", &mut plan),
            MappingOutcome::Applied | MappingOutcome::Archived
        ));
        assert_eq!(
            plan.site_text.get("site_name").map(String::as_str),
            Some("我的博客")
        );
        assert_eq!(
            plan.site_text.get("icp_text").map(String::as_str),
            Some("京ICP备xxx")
        );
    }

    #[test]
    fn email_keys_map_to_email_group_and_secret_is_written_not_recorded() {
        let mut plan = SettingsPlan::default();
        assert!(matches!(
            map_setting_item(1, "邮件设置", "address", "smtp.qq.com", &mut plan),
            MappingOutcome::Applied | MappingOutcome::Archived
        ));
        assert!(matches!(
            map_setting_item(2, "邮件设置", "port", "465", &mut plan),
            MappingOutcome::Applied | MappingOutcome::Archived
        ));
        assert!(matches!(
            map_setting_item(3, "邮件设置", "account", "a@qq.com", &mut plan),
            MappingOutcome::Applied | MappingOutcome::Archived
        ));
        assert!(matches!(
            map_setting_item(4, "邮件设置", "pwd", "s3cret", &mut plan),
            MappingOutcome::Applied | MappingOutcome::Archived
        ));
        assert!(matches!(
            map_setting_item(5, "邮件设置", "sender", "站长", &mut plan),
            MappingOutcome::Applied | MappingOutcome::Archived
        ));

        assert_eq!(plan.email.get("enabled"), Some(&Value::Bool(true)));
        assert_eq!(
            plan.email.get("smtp_host").and_then(Value::as_str),
            Some("smtp.qq.com")
        );
        assert_eq!(
            plan.email.get("smtp_port").and_then(Value::as_u64),
            Some(465)
        );
        assert_eq!(
            plan.email.get("from_name").and_then(Value::as_str),
            Some("站长")
        );
        // Secret 写入 payload 但不产生 Archive / skipped 记录。
        assert_eq!(
            plan.email.get("smtp_password").and_then(Value::as_str),
            Some("s3cret")
        );
        assert!(plan.archives.is_empty());
        assert!(plan.skipped_secrets.is_empty());
    }

    #[test]
    fn empty_smtp_address_does_not_enable_email() {
        let mut plan = SettingsPlan::default();
        assert!(matches!(
            map_setting_item(1, "邮件设置", "address", "  ", &mut plan),
            MappingOutcome::Applied | MappingOutcome::Archived
        ));
        assert!(plan.email.is_empty());
    }

    #[test]
    fn invalid_numeric_values_are_archived() {
        let mut plan = SettingsPlan::default();
        assert!(matches!(
            map_setting_item(1, "参数设置", "index_page_size", "abc", &mut plan),
            MappingOutcome::Applied | MappingOutcome::Archived
        ));
        assert!(matches!(
            map_setting_item(2, "参数设置", "index_page_size", "0", &mut plan),
            MappingOutcome::Applied | MappingOutcome::Archived
        ));
        assert_eq!(plan.archives.len(), 2);
        assert!(plan.site_int.is_empty());

        let mut plan = SettingsPlan::default();
        assert!(matches!(
            map_setting_item(3, "参数设置", "index_page_size", "20", &mut plan),
            MappingOutcome::Applied | MappingOutcome::Archived
        ));
        assert_eq!(plan.site_int.get("page_size_index"), Some(&20));
        assert!(matches!(
            map_setting_item(4, "参数设置", "archive_page_size", "15", &mut plan),
            MappingOutcome::Applied | MappingOutcome::Archived
        ));
        assert_eq!(plan.site_int.get("page_size_archive"), Some(&15));
        assert!(matches!(
            map_setting_item(5, "参数设置", "site_map_page_size", "30", &mut plan),
            MappingOutcome::Applied | MappingOutcome::Archived
        ));
        assert_eq!(plan.site_int.get("page_size_search"), Some(&30));
    }

    #[test]
    fn comment_policy_synthesized_from_legacy_switches() {
        let mut plan = SettingsPlan::default();
        assert!(matches!(
            map_setting_item(1, "评论设置", "is_on", "1", &mut plan),
            MappingOutcome::Applied | MappingOutcome::Archived
        ));
        assert!(matches!(
            map_setting_item(2, "评论设置", "is_review_on", "0", &mut plan),
            MappingOutcome::Applied | MappingOutcome::Archived
        ));
        assert!(matches!(
            map_setting_item(3, "评论设置", "page_size", "30", &mut plan),
            MappingOutcome::Applied | MappingOutcome::Archived
        ));
        plan.finalize();
        assert_eq!(
            plan.site_text.get("comment_policy").map(String::as_str),
            Some("auto_approve")
        );
        assert_eq!(plan.site_int.get("comments_per_page"), Some(&30));

        let mut plan = SettingsPlan::default();
        assert!(matches!(
            map_setting_item(1, "评论设置", "is_on", "0", &mut plan),
            MappingOutcome::Applied | MappingOutcome::Archived
        ));
        plan.finalize();
        assert_eq!(
            plan.site_text.get("comment_policy").map(String::as_str),
            Some("closed")
        );

        // 越界 page_size 进 Archive。
        let mut plan = SettingsPlan::default();
        assert!(matches!(
            map_setting_item(1, "评论设置", "page_size", "500", &mut plan),
            MappingOutcome::Applied | MappingOutcome::Archived
        ));
        assert_eq!(plan.archives.len(), 1);
        assert!(plan.site_int.is_empty());

        // 无评论设置时不写 comment_policy（保持目标默认 moderated）。
        let mut plan = SettingsPlan::default();
        plan.finalize();
        assert!(!plan.site_text.contains_key("comment_policy"));
    }

    #[test]
    fn unrecognized_keys_archived_unless_secret() {
        let mut plan = SettingsPlan::default();
        assert_eq!(
            map_setting_item(1, "社交信息", "github", "https://github.com/x", &mut plan),
            MappingOutcome::Unrecognized
        );
        assert_eq!(
            map_setting_item(2, "图床设置", "token", "secret-token", &mut plan),
            MappingOutcome::Unrecognized
        );
        assert_eq!(
            map_setting_item(3, "图床设置", "storage_type", "smms", &mut plan),
            MappingOutcome::Unrecognized
        );

        // 由 migrate 的兜底逻辑决定去向：非 Secret 进 Archive，Secret 只记名。
        assert!(is_secret_key("token"));
        assert!(is_secret_key("pwd"));
        assert!(!is_secret_key("github"));
    }
}
