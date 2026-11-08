use serde::Serialize;

/// 健康检查响应体；Liveness 与 Readiness 端点共用同一形状。
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub service: &'static str,
}

impl HealthResponse {
    /// 构造 Liveness 响应：进程能执行到这里即视为存活，不检查下游依赖。
    pub const fn live() -> Self {
        Self {
            status: "ok",
            service: "aries-server",
        }
    }
}
