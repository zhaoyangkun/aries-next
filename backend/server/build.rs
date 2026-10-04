//! Admin SPA 构建产物的编译期保障：`include_dir!("../../apps/admin/dist")` 要求目录在
//! 编译期存在，本地从未跑过 `pnpm --filter @aries/admin build` 时放占位 index.html，
//! 保证 `cargo test/clippy` 可以编译；Docker 构建会先产出真实 dist，占位不会生效。

use std::path::Path;

const PLACEHOLDER: &str = r#"<!doctype html>
<html lang="zh-CN">
  <head><meta charset="utf-8"><title>Aries Admin</title></head>
  <body>
    <p>Admin 前端尚未构建。请运行 <code>pnpm --filter @aries/admin build</code> 后重新编译服务端。</p>
  </body>
</html>
"#;

fn main() {
    let dist = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/admin/dist");
    if !dist.join("index.html").exists() {
        std::fs::create_dir_all(&dist).expect("create apps/admin/dist placeholder directory");
        std::fs::write(dist.join("index.html"), PLACEHOLDER)
            .expect("write apps/admin/dist placeholder index.html");
    }
}
