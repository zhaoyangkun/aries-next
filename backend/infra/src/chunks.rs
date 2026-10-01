//! AI 检索的 PostgreSQL 实现：文章内容块与向量存储。
//! `article_chunks.embedding` 为内置 `real[]`（不依赖 pgvector 扩展），维度不固定，
//! 兼容不同 Embedding 模型；余弦距离在应用层计算，语料量小时足够快。

use std::collections::HashMap;

use crate::logged::{logged_query, logged_query_as};
use aries_core::chunking::ContentChunk;
use aries_core::retrieval::{ChunkRepository, RelatedArticle, RetrievalError, RetrievedChunk};
use async_trait::async_trait;
use sqlx::{FromRow, PgPool};

#[derive(Clone)]
pub struct PostgresChunkRepository {
    pool: PgPool,
}

impl PostgresChunkRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// 余弦距离（1 - cosθ）；维度不匹配或任一方零范数时返回 None（该对不参与排序）。
fn cosine_distance(a: &[f32], b: &[f32]) -> Option<f32> {
    if a.len() != b.len() || a.is_empty() {
        return None;
    }
    let mut dot = 0.0_f64;
    let mut norm_a = 0.0_f64;
    let mut norm_b = 0.0_f64;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += f64::from(*x) * f64::from(*y);
        norm_a += f64::from(*x) * f64::from(*x);
        norm_b += f64::from(*y) * f64::from(*y);
    }
    if norm_a == 0.0 || norm_b == 0.0 {
        return None;
    }
    Some(1.0 - (dot / (norm_a.sqrt() * norm_b.sqrt())) as f32)
}

fn map_sqlx(error: sqlx::Error) -> RetrievalError {
    tracing::error!(error = %error, "chunk repository operation failed");
    RetrievalError::StoreUnavailable
}

#[derive(Debug, FromRow)]
struct RetrievedChunkRow {
    article_id: i64,
    slug: String,
    title: String,
    heading: String,
    content: String,
    embedding: Option<Vec<f32>>,
}

impl RetrievedChunkRow {
    fn into_retrieved(self, distance: f32) -> (f32, RetrievedChunk) {
        (
            distance,
            RetrievedChunk {
                article_id: self.article_id,
                article_slug: self.slug,
                article_title: self.title,
                heading: self.heading,
                content: self.content,
            },
        )
    }
}

#[derive(Debug, FromRow)]
struct RelatedArticleRow {
    slug: String,
    title: String,
    cover_url: Option<String>,
    published_at: Option<time::OffsetDateTime>,
    embedding: Option<Vec<f32>>,
}

#[async_trait]
impl ChunkRepository for PostgresChunkRepository {
    async fn replace_chunks(
        &self,
        article_id: i64,
        chunks: &[ContentChunk],
        embeddings: &[Vec<f32>],
    ) -> Result<(), RetrievalError> {
        if chunks.len() != embeddings.len() {
            tracing::error!(
                article_id,
                chunks = chunks.len(),
                embeddings = embeddings.len(),
                "replace_chunks received mismatched chunk/embedding counts"
            );
            return Err(RetrievalError::StoreUnavailable);
        }
        let mut transaction = self.pool.begin().await.map_err(map_sqlx)?;
        logged_query("DELETE FROM article_chunks WHERE article_id = $1")
            .bind(article_id)
            .execute(&mut *transaction)
            .await
            .map_err(map_sqlx)?;
        for (index, (chunk, embedding)) in chunks.iter().zip(embeddings.iter()).enumerate() {
            let chunk_index = i32::try_from(index).map_err(|_| {
                tracing::error!(article_id, chunks = chunks.len(), "chunk index overflow");
                RetrievalError::StoreUnavailable
            })?;
            logged_query(
                "INSERT INTO article_chunks (article_id, chunk_index, heading, content, embedding) \
                 VALUES ($1, $2, $3, $4, $5)",
            )
            .bind(article_id)
            .bind(chunk_index)
            .bind(&chunk.heading)
            .bind(&chunk.content)
            .bind(embedding)
            .execute(&mut *transaction)
            .await
            .map_err(map_sqlx)?;
        }
        transaction.commit().await.map_err(map_sqlx)
    }

    async fn delete_chunks(&self, article_id: i64) -> Result<(), RetrievalError> {
        logged_query("DELETE FROM article_chunks WHERE article_id = $1")
            .bind(article_id)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx)?;
        Ok(())
    }

    async fn nearest_chunks(
        &self,
        embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<RetrievedChunk>, RetrievalError> {
        let rows = logged_query_as::<RetrievedChunkRow>(
            "SELECT c.article_id, a.slug::text AS slug, a.title, c.heading, c.content, c.embedding \
             FROM article_chunks c JOIN articles a ON a.id = c.article_id \
             WHERE a.status = 'published' AND a.deleted_at IS NULL \
                 AND c.embedding IS NOT NULL",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;
        let mut ranked: Vec<(f32, RetrievedChunk)> = rows
            .into_iter()
            .filter_map(|row| {
                cosine_distance(embedding, row.embedding.as_deref()?)
                    .map(|distance| row.into_retrieved(distance))
            })
            .collect();
        ranked.sort_by(|a, b| a.0.total_cmp(&b.0));
        ranked.truncate(limit);
        Ok(ranked.into_iter().map(|(_, chunk)| chunk).collect())
    }

    async fn nearest_articles(
        &self,
        embedding: &[f32],
        exclude_article_id: i64,
        limit: usize,
    ) -> Result<Vec<RelatedArticle>, RetrievalError> {
        let rows = logged_query_as::<RelatedArticleRow>(
            "SELECT a.slug::text AS slug, a.title, a.cover_url, a.published_at, c.embedding \
             FROM article_chunks c JOIN articles a ON a.id = c.article_id \
             WHERE a.status = 'published' AND a.deleted_at IS NULL \
                 AND a.id <> $1 AND c.embedding IS NOT NULL",
        )
        .bind(exclude_article_id)
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;
        // 每篇文章取最近块的余弦距离作为文章距离。
        let mut by_article: HashMap<String, (f32, RelatedArticleRow)> = HashMap::new();
        for row in rows {
            let Some(distance) = row
                .embedding
                .as_deref()
                .and_then(|vector| cosine_distance(embedding, vector))
            else {
                continue;
            };
            let key = row.slug.clone();
            let replace = match by_article.get(&key) {
                Some((best, _)) => distance < *best,
                None => true,
            };
            if replace {
                by_article.insert(
                    key,
                    (
                        distance,
                        RelatedArticleRow {
                            embedding: None,
                            ..row
                        },
                    ),
                );
            }
        }
        let mut ranked: Vec<(f32, RelatedArticle)> = by_article
            .into_values()
            .map(|(distance, row)| {
                (
                    distance,
                    RelatedArticle {
                        slug: row.slug,
                        title: row.title,
                        cover_url: row.cover_url,
                        published_at: row.published_at,
                    },
                )
            })
            .collect();
        ranked.sort_by(|a, b| a.0.total_cmp(&b.0));
        ranked.truncate(limit);
        Ok(ranked.into_iter().map(|(_, article)| article).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_distance_orders_by_similarity() {
        let query = [1.0_f32, 0.0, 0.0];
        let closer = [0.9_f32, 0.1, 0.0];
        let farther = [0.2_f32, 0.8, 0.0];
        let close = cosine_distance(&query, &closer).unwrap();
        let far = cosine_distance(&query, &farther).unwrap();
        assert!(close < far);
        // 同向量为零距离。
        assert_eq!(cosine_distance(&query, &query), Some(0.0));
    }

    #[test]
    fn cosine_distance_rejects_mismatched_and_zero_norm() {
        assert_eq!(cosine_distance(&[1.0], &[1.0, 2.0]), None);
        assert_eq!(cosine_distance(&[1.0], &[]), None);
        assert_eq!(cosine_distance(&[0.0, 0.0], &[1.0, 0.0]), None);
    }
}
