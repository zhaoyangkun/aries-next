//! Archive JSONL 写入器：记录无法落入目标 Schema 的源数据，保证「不静默丢弃」。
//! 每行 `{"table", "id", "fields": {...}, "reason"}`。
//! 红线：不得写入密码 Hash、Secret 设置值与原始 UA（调用方负责过滤）。

use std::{
    collections::BTreeMap,
    fs::File,
    io::{BufWriter, Write},
    path::Path,
};

use serde::Serialize;

#[derive(Debug, Serialize)]
struct ArchiveLine<'a> {
    table: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<i64>,
    fields: serde_json::Value,
    reason: &'a str,
    /// 整行未迁入时为 true；字段级归档省略此键。
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    whole_row: bool,
}

pub struct ArchiveWriter {
    writer: BufWriter<File>,
    counts: BTreeMap<String, u64>,
}

impl ArchiveWriter {
    pub fn create(path: &Path) -> std::io::Result<Self> {
        Ok(Self {
            writer: BufWriter::new(File::create(path)?),
            counts: BTreeMap::new(),
        })
    }

    pub fn record(
        &mut self,
        table: &str,
        id: Option<i64>,
        fields: serde_json::Value,
        reason: &str,
        whole_row: bool,
    ) -> std::io::Result<()> {
        let line = ArchiveLine {
            table,
            id,
            fields,
            reason,
            whole_row,
        };
        serde_json::to_writer(&mut self.writer, &line)?;
        self.writer.write_all(b"\n")?;
        *self.counts.entry(table.to_owned()).or_default() += 1;
        Ok(())
    }

    pub fn finish(mut self) -> std::io::Result<()> {
        self.writer.flush()
    }
}

/// Archive 文件索引：每表总行数 + 整行归档（whole_row）的 id 集合。
pub struct ArchiveIndex {
    pub counts: BTreeMap<String, u64>,
    pub whole_row_ids: BTreeMap<String, std::collections::HashSet<i64>>,
}

/// 解析已有 Archive 文件（validate 对账用）。
pub fn load_index(path: &Path) -> anyhow::Result<ArchiveIndex> {
    let content = std::fs::read_to_string(path)?;
    let mut counts = BTreeMap::new();
    let mut whole_rows: BTreeMap<String, std::collections::HashSet<i64>> = BTreeMap::new();
    for line in content.lines().filter(|line| !line.trim().is_empty()) {
        let value: serde_json::Value = serde_json::from_str(line)?;
        let table = value
            .get("table")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("<unknown>")
            .to_owned();
        *counts.entry(table.clone()).or_default() += 1;
        if value
            .get("whole_row")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
            && let Some(id) = value.get("id").and_then(serde_json::Value::as_i64)
        {
            whole_rows.entry(table).or_default().insert(id);
        }
    }
    Ok(ArchiveIndex {
        counts,
        whole_row_ids: whole_rows,
    })
}
