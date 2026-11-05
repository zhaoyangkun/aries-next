-- setting_groups 增加 social 分组：社交链接列表（name/url/icon），公开端 footer 展示。
-- 沿用 ai 分组的迁移做法：重建 grp CHECK 约束 + 预插默认行。

ALTER TABLE setting_groups
    DROP CONSTRAINT IF EXISTS setting_groups_grp_check;

ALTER TABLE setting_groups
    ADD CONSTRAINT setting_groups_grp_check
    CHECK (grp IN ('appearance', 'email', 'integrations', 'ai', 'social'));

INSERT INTO setting_groups (grp)
VALUES ('social')
ON CONFLICT (grp) DO NOTHING;
