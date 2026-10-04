//! OpenAPI 文档：utoipa 注解生成的 API 契约（试点阶段仅覆盖已迁移模块）。
//!
//! 迁移策略：端点逐个从手写 `docs/openapi.yaml` 迁移到代码注解，
//! 已迁移部分的机器生成物落在 `docs/openapi.generated.yaml`，
//! 由 CI 漂移检查（导出结果与提交文件逐字节 diff）保证代码与契约永不脱节；
//! 全部迁移完成后，生成物取代手写 `docs/openapi.yaml`。

use utoipa::openapi::security::{ApiKey, ApiKeyValue, SecurityScheme};
use utoipa::{Modify, OpenApi};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Aries Next API",
        version = "0.1.0",
        description = "Aries Next 博客系统 API（utoipa 自动生成的迁移中子集，完整契约见 docs/openapi.yaml）",
    ),
    servers((url = "/")),
    paths(crate::http::ai::list_ai_models),
    components(schemas(crate::http::ai::AiModelsResponse)),
    modifiers(&SecurityAddon),
)]
pub struct ApiDoc;

/// 统一注册安全方案：Admin Session 为 HttpOnly Cookie（Path 限定 /api/admin）。
struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        openapi
            .components
            .get_or_insert_with(Default::default)
            .add_security_scheme(
                "cookieAuth",
                SecurityScheme::ApiKey(ApiKey::Cookie(ApiKeyValue::new("aries_admin_session"))),
            );
    }
}

/// 序列化为 YAML（导出脚本与漂移检查共用）。
pub fn to_yaml() -> anyhow::Result<String> {
    Ok(ApiDoc::openapi().to_yaml()?)
}
