-- 公开搜索（search_public_articles）的 FTS 条件即时计算 to_tsvector，无索引、全表扫描。
-- 建立与查询表达式完全一致的 GIN 表达式索引；部分索引谓词与查询的
-- WHERE 条件（status = 'published' AND deleted_at IS NULL）保持一致，使 Planner 可走索引。
-- SQLx Migration 在事务内运行，不能用 CREATE INDEX CONCURRENTLY；
-- IF NOT EXISTS 保证重复执行安全。
CREATE INDEX IF NOT EXISTS articles_fts_search_idx
    ON articles USING GIN (
        to_tsvector(
            'simple',
            coalesce(title, '') || ' ' || coalesce(summary, '')
                || ' ' || coalesce(markdown_source, '')
        )
    )
    WHERE status = 'published' AND deleted_at IS NULL;
