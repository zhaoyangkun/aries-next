use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub service: &'static str,
}

impl HealthResponse {
    pub const fn live() -> Self {
        Self {
            status: "ok",
            service: "aries-server",
        }
    }
}
