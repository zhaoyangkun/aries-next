-- 重建 articles.comment_count：该列此前没有任何运行期代码维护，
-- 唯一写入方是 migrator 的数据回填，导致公开详情接口透出错误计数。
-- 自 20260912 起由 aries-infra 的评论 Repository 在审核/删除等状态变更事务内重建本列；
-- 本 Migration 负责把存量数据一次性校正为真实 approved 评论数。
UPDATE articles
SET comment_count = (
    SELECT COUNT(*) FROM comments
    WHERE comments.target_type = 'article'
      AND comments.target_id = articles.id
      AND comments.status = 'approved'
      AND comments.deleted_at IS NULL
);
