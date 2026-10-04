//! 导出 utoipa 自动生成的 OpenAPI 契约（已迁移模块子集）为 YAML 到 stdout。
//!
//! 用法（仓库根目录）：
//!   cargo run -p aries-server --bin openapi-export > docs/openapi.generated.yaml
//!
//! 该文件由 CI 漂移检查守护：改了注解必须重新导出并提交，否则构建失败。
//! 全部端点迁移完成前，完整权威契约仍是手写的 docs/openapi.yaml。

fn main() -> anyhow::Result<()> {
    print!("{}", aries_server::openapi::to_yaml()?);
    Ok(())
}
