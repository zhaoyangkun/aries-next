//! AI 检索（相关文章 / 对话式搜索）的存储契约：core 定义 trait，infra 提供 PostgreSQL 实现。

use async_trait::async_trait;
use thiserror::Error;
use time::OffsetDateTime;

use crate::chunking::ContentChunk;

/// 命中的内容块，附带所属文章信息（对话式搜索引用需要标题与链接）。
#[derive(Debug, Clone)]
pub struct RetrievedChunk {
    pub article_id: i64,
    pub article_slug: String,
    pub article_title: String,
    pub heading: String,
    pub content: String,
}

/// 相关文章条目：公开端点的最小字段集。
#[derive(Debug, Clone)]
pub struct RelatedArticle {
    pub slug: String,
    pub title: String,
    pub cover_url: Option<String>,
    pub published_at: Option<OffsetDateTime>,
}

#[derive(Debug, Error)]
pub enum RetrievalError {
    #[error("retrieval store unavailable")]
    StoreUnavailable,
}

#[async_trait]
pub trait ChunkRepository: Send + Sync {
    /// 事务内删除文章旧块并插入新块；`embeddings` 与 `chunks` 等长且顺序一致。
    async fn replace_chunks(
        &self,
        article_id: i64,
        chunks: &[ContentChunk],
        embeddings: &[Vec<f32>],
    ) -> Result<(), RetrievalError>;

    /// 删除文章全部内容块（取消发布 / 删除文章时调用）。
    async fn delete_chunks(&self, article_id: i64) -> Result<(), RetrievalError>;

    /// 全文范围内按余弦距离取最近的已发布文章内容块（对话式搜索检索）。
    async fn nearest_chunks(
        &self,
        embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<RetrievedChunk>, RetrievalError>;

    /// 与目标文章最相近的其他已发布文章（相关文章推荐）。
    async fn nearest_articles(
        &self,
        embedding: &[f32],
        exclude_article_id: i64,
        limit: usize,
    ) -> Result<Vec<RelatedArticle>, RetrievalError>;
}
