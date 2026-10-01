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
    pub fn new(root: impl Into<PathBuf>, public_base_url: impl Into<String>) -> Self {
        Self {
            root: root.into(),
            public_base_url: public_base_url.into(),
        }
    }

    fn resolve(&self, object_key: &str) -> Result<PathBuf, MediaError> {
        validate_object_key(object_key)?;
        Ok(self.root.join(object_key))
    }
}

#[async_trait]
impl MediaStorage for LocalMediaStorage {
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

    fn url_for(&self, object_key: &str) -> String {
        format!(
            "{}/{}",
            self.public_base_url.trim_end_matches('/'),
            object_key
        )
    }

    fn serves_files_locally(&self) -> bool {
        true
    }
}
