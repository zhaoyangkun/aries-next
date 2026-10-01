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

    pub fn table(&mut self, name: &str) -> &mut TableReport {
        self.tables.entry(name.to_owned()).or_default()
    }

    pub fn note(&mut self, message: impl Into<String>) {
        self.notes.push(message.into());
    }

    pub fn finish(&mut self, started: std::time::Instant) {
        self.finished_at = OffsetDateTime::now_utc();
        self.duration_ms = started.elapsed().as_millis() as i64;
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let file = std::fs::File::create(path)?;
        serde_json::to_writer_pretty(file, self)?;
        Ok(())
    }

    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let file = std::fs::File::open(path)?;
        Ok(serde_json::from_reader(file)?)
    }
}
