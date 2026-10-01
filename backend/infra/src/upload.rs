//! 上传文件的纯校验逻辑：扩展名白名单、MIME 与 Magic Bytes 交叉验证、尺寸探测。
//! 放在 infra 层是因为它与存储实现共享“什么文件可以入库”的判定标准，且便于单元测试。

use sha2::{Digest, Sha256};
use thiserror::Error;
use time::OffsetDateTime;
use uuid::Uuid;

/// 单文件上限 5MB，与 API Contract 一致。
pub const MAX_FILE_BYTES: usize = 5 * 1024 * 1024;
/// 单批上传最多 5 个文件，对齐旧版行为。
pub const MAX_BATCH_FILES: usize = 5;

/// 允许的图片类型；扩展名、MIME 与 Magic Bytes 三者必须互相印证，
/// 防止把伪装成图片的脚本或文本写入媒体库。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageKind {
    Jpeg,
    Png,
    Gif,
    Bmp,
    Webp,
}

impl ImageKind {
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Png => "png",
            Self::Gif => "gif",
            Self::Bmp => "bmp",
            Self::Webp => "webp",
        }
    }

    pub const fn mime(self) -> &'static str {
        match self {
            Self::Jpeg => "image/jpeg",
            Self::Png => "image/png",
            Self::Gif => "image/gif",
            Self::Bmp => "image/bmp",
            Self::Webp => "image/webp",
        }
    }

    /// 扩展名白名单匹配（大小写不敏感）；jpeg 是 jpg 的常见别名。
    pub fn from_extension(extension: &str) -> Option<Self> {
        match extension.to_ascii_lowercase().as_str() {
            "jpg" | "jpeg" => Some(Self::Jpeg),
            "png" => Some(Self::Png),
            "gif" => Some(Self::Gif),
            "bmp" => Some(Self::Bmp),
            "webp" => Some(Self::Webp),
            _ => None,
        }
    }

    /// 按 Magic Bytes 识别真实格式，不信任客户端声明。
    pub fn detect(bytes: &[u8]) -> Option<Self> {
        if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            return Some(Self::Jpeg);
        }
        if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
            return Some(Self::Png);
        }
        if bytes.starts_with(b"GIF8") {
            return Some(Self::Gif);
        }
        if bytes.starts_with(b"BM") {
            return Some(Self::Bmp);
        }
        if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
            return Some(Self::Webp);
        }
        None
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum UploadError {
    #[error("file is empty")]
    Empty,
    #[error("file exceeds the 5MB size limit")]
    TooLarge,
    #[error("file extension is not allowed")]
    UnsupportedExtension,
    #[error("file content is not a supported image")]
    UnsupportedContent,
    #[error("file extension does not match its content")]
    ContentMismatch,
    #[error("declared MIME type does not match the file content")]
    MimeMismatch,
}

/// 校验通过的上传结果：后续入库所需字段一次性算齐，避免重复读取字节。
#[derive(Debug, Clone, PartialEq)]
pub struct ValidatedUpload {
    pub kind: ImageKind,
    pub mime: String,
    pub size_bytes: i64,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub sha256: String,
}

/// 从原始文件名取扩展名并落入白名单；文件名其余部分一律不信任。
pub fn sanitize_extension(original_name: &str) -> Option<&'static str> {
    let extension = original_name.rsplit('.').next()?;
    if extension.len() == original_name.len() || extension.len() > 5 {
        return None;
    }
    ImageKind::from_extension(extension).map(ImageKind::extension)
}

/// 生成 Object Key：`yyyy/mm/<uuid v7>.<ext>`。
/// 不使用用户文件名，避免路径注入与同名覆盖；按月份分目录防止单目录文件过多。
pub fn generate_object_key(extension: &str) -> String {
    let now = OffsetDateTime::now_utc();
    format!(
        "{:04}/{:02}/{}.{}",
        now.year(),
        u8::from(now.month()),
        Uuid::now_v7().simple(),
        extension
    )
}

/// 探测图片尺寸；失败返回 `None`，不阻断上传（尺寸由后台任务补探测）。
pub fn probe_dimensions(bytes: &[u8]) -> Option<(i32, i32)> {
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let (width, height) = reader.into_dimensions().ok()?;
    Some((i32::try_from(width).ok()?, i32::try_from(height).ok()?))
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// 完整校验一个上传文件：大小、扩展名白名单、Magic Bytes、声明 MIME 交叉验证。
pub fn validate_upload(
    original_name: &str,
    declared_mime: Option<&str>,
    bytes: &[u8],
) -> Result<ValidatedUpload, UploadError> {
    if bytes.is_empty() {
        return Err(UploadError::Empty);
    }
    if bytes.len() > MAX_FILE_BYTES {
        return Err(UploadError::TooLarge);
    }
    let extension = sanitize_extension(original_name).ok_or(UploadError::UnsupportedExtension)?;
    let expected = ImageKind::from_extension(extension).ok_or(UploadError::UnsupportedExtension)?;
    let detected = ImageKind::detect(bytes).ok_or(UploadError::UnsupportedContent)?;
    if detected != expected {
        return Err(UploadError::ContentMismatch);
    }
    if let Some(mime) = declared_mime.filter(|value| !value.is_empty()) {
        if !mime.eq_ignore_ascii_case(detected.mime()) {
            return Err(UploadError::MimeMismatch);
        }
    }
    let (width, height) = probe_dimensions(bytes).map_or((None, None), |(w, h)| (Some(w), Some(h)));
    Ok(ValidatedUpload {
        kind: detected,
        mime: detected.mime().to_owned(),
        size_bytes: i64::try_from(bytes.len()).map_err(|_| UploadError::TooLarge)?,
        width,
        height,
        sha256: sha256_hex(bytes),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG_MAGIC: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

    #[test]
    fn magic_bytes_detect_all_supported_formats() {
        assert_eq!(
            ImageKind::detect(&[0xFF, 0xD8, 0xFF, 0xE0]),
            Some(ImageKind::Jpeg)
        );
        assert_eq!(ImageKind::detect(&PNG_MAGIC), Some(ImageKind::Png));
        assert_eq!(ImageKind::detect(b"GIF89a..."), Some(ImageKind::Gif));
        assert_eq!(ImageKind::detect(b"BM...."), Some(ImageKind::Bmp));
        assert_eq!(
            ImageKind::detect(b"RIFF\x04\x00\x00\x00WEBPVP8 "),
            Some(ImageKind::Webp)
        );
        assert_eq!(ImageKind::detect(b"plain text"), None);
    }

    #[test]
    fn extension_sanitization_rejects_missing_and_unknown_extensions() {
        assert_eq!(sanitize_extension("photo.JPG"), Some("jpg"));
        assert_eq!(sanitize_extension("photo.jpeg"), Some("jpg"));
        assert_eq!(sanitize_extension("no-extension"), None);
        assert_eq!(sanitize_extension("script.svg"), None);
    }

    #[test]
    fn object_key_uses_month_directory_and_never_user_input() {
        let key = generate_object_key("png");
        let mut segments = key.split('/');
        assert!(segments.next().is_some_and(|year| year.len() == 4));
        assert!(segments.next().is_some_and(|month| month.len() == 2));
        let file = segments.next().unwrap_or_default();
        assert!(file.ends_with(".png") && !file.contains(".."));
        assert!(segments.next().is_none());
        assert_ne!(generate_object_key("png"), key);
    }

    #[test]
    fn upload_validation_rejects_disguised_text_file() {
        // 扩展名伪装成 png 的文本文件必须被拒绝。
        let result = validate_upload("fake.png", Some("image/png"), b"hello world");
        assert_eq!(result, Err(UploadError::UnsupportedContent));

        let mismatch = validate_upload("fake.jpg", Some("image/jpeg"), &PNG_MAGIC);
        assert_eq!(mismatch, Err(UploadError::ContentMismatch));

        let wrong_mime = validate_upload("real.png", Some("image/gif"), &PNG_MAGIC);
        assert_eq!(wrong_mime, Err(UploadError::MimeMismatch));
    }

    #[test]
    fn upload_validation_accepts_real_png_without_dimensions_blocking() {
        let validated = validate_upload("real.png", Some("image/png"), &PNG_MAGIC)
            .expect("valid png magic must pass");
        assert_eq!(validated.mime, "image/png");
        // 只有 Magic Bytes 的伪 PNG 无法探测尺寸，尺寸允许为空。
        assert_eq!(validated.width, None);
    }
}
