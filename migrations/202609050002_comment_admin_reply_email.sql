-- 管理员回复没有访客 Email（`create_admin_reply` 插入空串）。
-- 原 `202609050001` 的约束要求 email 至少 1 字符，与管理员回复冲突；
-- 这里放宽为允许空值。访客提交的 email 非空校验由 Public 提交端点负责（Phase 05 第二批）。
ALTER TABLE comments
    DROP CONSTRAINT IF EXISTS comments_author_email_check;

ALTER TABLE comments
    ADD CONSTRAINT comments_author_email_check
        CHECK (char_length(author_email) BETWEEN 0 AND 254);
