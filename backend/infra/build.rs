// `sqlx::migrate!()` 宏在本 crate（aries-infra）编译期读取 migrations 目录并嵌入 SQL，
// 但宏的文件变更追踪默认未启用（cfg procmacro2_semver_exempt），仅新增 Migration
// 文件不会触发本 crate 重编译，导致运行中的二进制缺少新表结构（曾造成
// server_logs 表未创建）。这里显式声明目录依赖，新增/修改 Migration 后 cargo
// 必然重建 aries-infra。
fn main() {
    println!("cargo:rerun-if-changed=../../migrations");
}
