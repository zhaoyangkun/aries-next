-- 站点设置新增公安联网备案号（网安备号），与 ICP 备案号并列展示在公开端 Footer。
-- 存量行默认为空串：footer 按空值隐藏该备案链接。

ALTER TABLE site_settings
    ADD COLUMN police_text text NOT NULL DEFAULT '';
