-- 公开站点信息需要建站时间（footer「本站已运行 X 天」）。
-- 存量行用最早发布文章时间回填：能代表真实建站时间；无文章时退化为迁移执行时间。

ALTER TABLE site_settings
    ADD COLUMN created_at timestamptz NOT NULL DEFAULT now();

UPDATE site_settings
SET created_at = COALESCE((SELECT min(published_at) FROM articles), now())
WHERE id = 1;
