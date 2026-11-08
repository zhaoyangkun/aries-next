//! 迁移运行报告：每张表的 migrated / skipped / repaired / failed / archived 计数与备注，
//! 结束时打印并落盘为 JSON。Report 不含密码 Hash、Secret 值与原始 UA。

use std::{collections::BTreeMap, path::Path};

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct TableReport {
    #[serde(default)]
    pub source_rows: i64,
    #[serde(default)]
    pub migrated: i64,
    #[serde(default)]
    pub skipped: i64,
    #[serde(default)]
    pub repaired: i64,
    #[serde(default)]
    pub failed: i64,
    #[serde(default)]
    pub archived: i64,
    /// 其中「整行未迁入」的条目数；对账公式：源行数 = migrated + skipped + archived_rows + failed。
    /// archived 含字段级条目（如旧渲染 HTML），不参与对账。
    #[serde(default)]
    pub archived_rows: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

impl TableReport {
    /// 追加表级备注。
    pub fn note(&mut self, message: impl Into<String>) {
        self.notes.push(message.into());
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RunReport {
    pub run_id: Uuid,
    pub command: String,
    #[serde(with = "time::serde::rfc3339")]
    pub started_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub finished_at: OffsetDateTime,
    pub duration_ms: i64,
    #[serde(default)]
    pub tables: BTreeMap<String, TableReport>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

impl RunReport {
    /// 创建运行报告并返回计时起点。
    pub fn start(command: &str) -> (Self, std::time::Instant) {
        (
            Self {
                run_id: Uuid::now_v7(),
                command: command.to_owned(),
                started_at: OffsetDateTime::now_utc(),
                finished_at: OffsetDateTime::now_utc(),
                duration_ms: 0,
                tables: BTreeMap::new(),
                notes: Vec::new(),
            },
            std::time::Instant::now(),
        )
    }

    /// 取（或建）指定表的报告条目。
    pub fn table(&mut self, name: &str) -> &mut TableReport {
        self.tables.entry(name.to_owned()).or_default()
    }

    /// 追加运行级备注。
    pub fn note(&mut self, message: impl Into<String>) {
        self.notes.push(message.into());
    }

    /// 填充结束时间与耗时。
    pub fn finish(&mut self, started: std::time::Instant) {
        self.finished_at = OffsetDateTime::now_utc();
        self.duration_ms = started.elapsed().as_millis() as i64;
    }

    /// 落盘为 JSON（不含密码 Hash/Secret/原始 UA）。
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let file = std::fs::File::create(path)?;
        serde_json::to_writer_pretty(file, self)?;
        Ok(())
    }

    /// 从磁盘加载历史报告。
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let file = std::fs::File::open(path)?;
        Ok(serde_json::from_reader(file)?)
    }
}
