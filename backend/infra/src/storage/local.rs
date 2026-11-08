use std::path::PathBuf;

use aries_core::media::{MediaError, MediaStorage};
use async_trait::async_trait;

use super::validate_object_key;

/// 本地磁盘存储：文件落在 `root/<object_key>`，公开 URL 为 `public_base_url/<object_key>`。
pub struct LocalMediaStorage {
    root: PathBuf,
    public_base_url: String,
}

impl LocalMediaStorage {
    /// 本地磁盘存储：root 为文件根目录，public_base_url 用于拼对外 URL。
    pub fn new(root: impl Into<PathBuf>, public_base_url: impl Into<String>) -> Self {
        Self {
            root: root.into(),
            public_base_url: public_base_url.into(),
        }
    }

    /// object_key → 磁盘路径（key 由 generate_object_key 生成，不含用户输入的目录穿越成分）。
    fn resolve(&self, object_key: &str) -> Result<PathBuf, MediaError> {
        validate_object_key(object_key)?;
        Ok(self.root.join(object_key))
    }
}

#[async_trait]
impl MediaStorage for LocalMediaStorage {
    /// 写入文件（父目录自动创建），返回对外 URL。
    async fn put(&self, object_key: &str, bytes: &[u8], _mime: &str) -> Result<String, MediaError> {
        let path = self.resolve(object_key)?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|error| {
                tracing::error!(error = %error, "failed to create media directory");
                MediaError::StoreUnavailable
            })?;
        }
        tokio::fs::write(&path, bytes).await.map_err(|error| {
            tracing::error!(error = %error, "failed to write media file");
            MediaError::StoreUnavailable
        })?;
        Ok(self.url_for(object_key))
    }

    /// 读取文件；不存在返回 None。
    async fn get(&self, object_key: &str) -> Result<Option<Vec<u8>>, MediaError> {
        let path = self.resolve(object_key)?;
        match tokio::fs::read(&path).await {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => {
                tracing::error!(error = %error, "failed to read media file");
                Err(MediaError::StoreUnavailable)
            }
        }
    }

    /// 删除文件（不存在不报错）。
    async fn delete(&self, object_key: &str) -> Result<(), MediaError> {
        let path = self.resolve(object_key)?;
        match tokio::fs::remove_file(&path).await {
            // 文件不存在视为删除成功，清理任务因此可以安全重试。
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => {
                tracing::error!(error = %error, "failed to delete media file");
                Err(MediaError::StoreUnavailable)
            }
        }
    }

    /// 对外 URL：public_base_url + object_key。
    fn url_for(&self, object_key: &str) -> String {
        format!(
            "{}/{}",
            self.public_base_url.trim_end_matches('/'),
            object_key
        )
    }

    /// 本地存储由 Server 直接提供文件服务（区别于 S3 的外部 URL）。
    fn serves_files_locally(&self) -> bool {
        true
    }
}
