//! 媒体存储后端：本地磁盘与 S3 兼容对象存储。

pub mod local;
pub mod s3;

use std::sync::Arc;

use anyhow::bail;
use aries_core::media::{MediaError, MediaStorage};

pub use local::LocalMediaStorage;
pub use s3::S3MediaStorage;

pub const DEFAULT_PUBLIC_BASE_URL: &str = "/api/media/files";
pub const DEFAULT_LOCAL_DIR: &str = "./data/media";

/// Object Key 安全校验：拒绝路径穿越与绝对路径。
/// Key 由服务端生成，这里做纵深防御；静态文件路由也复用同一校验。
pub fn validate_object_key(key: &str) -> Result<(), MediaError> {
    let valid = !key.is_empty()
        && !key.starts_with('/')
        && !key.contains('\\')
        && !key
            .split('/')
            .any(|segment| segment.is_empty() || segment == "..");
    if valid {
        Ok(())
    } else {
        Err(MediaError::Conflict)
    }
}

/// 按 `MEDIA_PROVIDER` 组装存储后端（默认 `local`）。
pub fn storage_from_env() -> anyhow::Result<Arc<dyn MediaStorage>> {
    let provider = std::env::var("MEDIA_PROVIDER").unwrap_or_else(|_| "local".to_owned());
    match provider.as_str() {
        "local" => {
            let dir =
                std::env::var("MEDIA_LOCAL_DIR").unwrap_or_else(|_| DEFAULT_LOCAL_DIR.to_owned());
            let base_url = std::env::var("MEDIA_PUBLIC_BASE_URL")
                .unwrap_or_else(|_| DEFAULT_PUBLIC_BASE_URL.to_owned());
            Ok(Arc::new(LocalMediaStorage::new(dir, base_url)))
        }
        "s3" => Ok(Arc::new(S3MediaStorage::from_env()?)),
        _ => bail!("MEDIA_PROVIDER must be local or s3"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_key_rejects_traversal_and_absolute_paths() {
        assert!(validate_object_key("2026/08/abc.png").is_ok());
        assert!(validate_object_key("../secret").is_err());
        assert!(validate_object_key("2026/../../secret").is_err());
        assert!(validate_object_key("/etc/passwd").is_err());
        assert!(validate_object_key("2026\\08\\x.png").is_err());
        assert!(validate_object_key("").is_err());
        assert!(validate_object_key("2026//x.png").is_err());
    }
}
