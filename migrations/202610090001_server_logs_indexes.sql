-- server_logs 查询索引补强：
-- 1) request_id 链路追踪查询（部分索引，只为有 request_id 的行付写入成本）；
-- 2) (ts DESC, id DESC) 复合索引服务列表排序 ORDER BY ts DESC, id DESC
--    与上下文模式（around_id 锚点）的 (ts, id) 元组比较。
-- 复合索引同样可服务 ts 单列范围扫描，原 server_logs_ts_idx 因此冗余，一并删除。

DROP INDEX server_logs_ts_idx;

CREATE INDEX server_logs_request_id_idx ON server_logs (request_id) WHERE request_id IS NOT NULL;
CREATE INDEX server_logs_ts_id_idx ON server_logs (ts DESC, id DESC);
