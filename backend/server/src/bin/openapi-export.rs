//! 导出 utoipa 注解生成的 OpenAPI 契约为 YAML 到 stdout。
//!
//! docs/openapi.yaml 即由本命令生成（机器生成物，禁止手改），同步命令：
//!   pnpm sync:api   # 重新导出 docs/openapi.yaml 并重新生成 packages/api-client 类型
//!
//! 漂移由契约测试 exported_yaml_matches_committed_generated_file 守护：
//! 改了注解必须重新同步并提交，否则测试失败。

fn main() -> anyhow::Result<()> {
    print!("{}", aries_server::openapi::to_yaml()?);
    Ok(())
}
