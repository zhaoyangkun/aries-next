use anyhow::Context;
use aries_core::media::{MediaError, MediaStorage};
use async_trait::async_trait;
use s3::{Bucket, Region, creds::Credentials};

use super::validate_object_key;

/// S3 兼容对象存储（AWS S3 / MinIO / R2），配置来自 `S3_*` 环境变量。
pub struct S3MediaStorage {
    bucket: Box<Bucket>,
    public_base_url: String,
}

impl S3MediaStorage {
    /// 从环境变量装配 S3 Client（endpoint/bucket/credentials/region）。
    pub fn from_env() -> anyhow::Result<Self> {
        let endpoint = required_env("S3_ENDPOINT")?;
        let bucket_name = required_env("S3_BUCKET")?;
        let access_key = required_env("S3_ACCESS_KEY")?;
        let secret_key = required_env("S3_SECRET_KEY")?;
        let region = std::env::var("S3_REGION").unwrap_or_else(|_| "us-east-1".to_owned());
        let public_base_url = required_env("S3_PUBLIC_BASE_URL")?;

        let credentials = Credentials::new(Some(&access_key), Some(&secret_key), None, None, None)
            .context("invalid S3 credentials")?;
        // Path Style 兼容 MinIO 等自建对象存储；公开访问走独立的 Public Base URL。
        let bucket = Bucket::new(
            &bucket_name,
            Region::Custom { region, endpoint },
            credentials,
        )
        .context("invalid S3 bucket configuration")?
        .with_path_style();
        Ok(Self {
            bucket,
            public_base_url,
        })
    }
}

#[async_trait]
impl MediaStorage for S3MediaStorage {
    /// 上传对象（按 mime 设 Content-Type），返回对外 URL。
    async fn put(&self, object_key: &str, bytes: &[u8], mime: &str) -> Result<String, MediaError> {
        validate_object_key(object_key)?;
        let response = self
            .bucket
            .put_object_with_content_type(object_key, bytes, mime)
            .await
            .map_err(map_s3)?;
        if response.status_code() >= 300 {
            tracing::error!(status = response.status_code(), "s3 put_object failed");
            return Err(MediaError::StoreUnavailable);
        }
        Ok(self.url_for(object_key))
    }

    /// 下载对象；不存在返回 None。
    async fn get(&self, object_key: &str) -> Result<Option<Vec<u8>>, MediaError> {
        validate_object_key(object_key)?;
        let response = self.bucket.get_object(object_key).await.map_err(map_s3)?;
        if response.status_code() == 404 {
            return Ok(None);
        }
        if response.status_code() >= 300 {
            tracing::error!(status = response.status_code(), "s3 get_object failed");
            return Err(MediaError::StoreUnavailable);
        }
        Ok(Some(response.bytes().to_vec()))
    }

    /// 删除对象。
    async fn delete(&self, object_key: &str) -> Result<(), MediaError> {
        validate_object_key(object_key)?;
        let response = self
            .bucket
            .delete_object(object_key)
            .await
            .map_err(map_s3)?;
        // 204 与 404 都视为删除成功，清理任务可安全重试。
        if response.status_code() >= 300 && response.status_code() != 404 {
            tracing::error!(status = response.status_code(), "s3 delete_object failed");
            return Err(MediaError::StoreUnavailable);
        }
        Ok(())
    }

    /// 对外 URL：PUBLIC_BASE_URL + object_key（S3 文件不经由本站服务）。
    fn url_for(&self, object_key: &str) -> String {
        format!(
            "{}/{}",
            self.public_base_url.trim_end_matches('/'),
            object_key
        )
    }

    /// S3 存储不由 Server 本地服务文件。
    fn serves_files_locally(&self) -> bool {
        false
    }
}

/// 读取必填环境变量。
fn required_env(name: &str) -> anyhow::Result<String> {
    std::env::var(name).with_context(|| format!("missing required environment variable {name}"))
}

/// S3 错误映射：NotFound 归 None/NotFound，其余记日志归存储故障。
fn map_s3(error: s3::error::S3Error) -> MediaError {
    tracing::error!(error = %error, "s3 storage operation failed");
    MediaError::StoreUnavailable
}
