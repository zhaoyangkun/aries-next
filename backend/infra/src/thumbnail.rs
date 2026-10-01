//! 按需缩略图：媒体文件服务 `GET /api/media/files/{*path}?w=<width>` 背后的实现。
//!
//! 原图不可变，因此缩略图缓存条目同样不可变：首次请求解码 → 等比缩到目标宽（不放大）
//! → 重编码 → 写入磁盘缓存（临时文件 + rename，避免并发请求读到写了一半的文件），
//! 后续请求直接命中缓存。缓存目录与 DB 跟踪的媒体资产完全隔离，`media_cleanup` 不会触碰。

use std::io::Cursor;
use std::path::{Path, PathBuf};

use image::{DynamicImage, ImageFormat};

/// 缩略图请求的最小目标宽度（防止无意义的微缩请求）。
pub const MIN_WIDTH: u32 = 16;
/// 缩略图请求的最大目标宽度（封面展示场景远用不到更大值）。
pub const MAX_WIDTH: u32 = 1200;

const DEFAULT_LOCAL_DIR: &str = "./data/media";

/// 解析 `?w=` 查询参数：非数字或越界返回 `None`，由调用方回退原图。
pub fn parse_width(raw: Option<&str>) -> Option<u32> {
    let width: u32 = raw?.parse().ok()?;
    (MIN_WIDTH..=MAX_WIDTH).contains(&width).then_some(width)
}

/// 该 MIME 是否支持生成缩略图。GIF 排除在外以保留动画；SVG/ICO 等 `image` crate 不支持。
pub fn is_resizable_image(mime: &str) -> bool {
    matches!(mime, "image/jpeg" | "image/png" | "image/webp")
}

/// 缩略图磁盘缓存根目录：`MEDIA_THUMB_DIR` 覆盖，默认 `<MEDIA_LOCAL_DIR>/.thumbs`。
pub fn cache_root() -> PathBuf {
    if let Ok(dir) = std::env::var("MEDIA_THUMB_DIR") {
        return PathBuf::from(dir);
    }
    let base = std::env::var("MEDIA_LOCAL_DIR").unwrap_or_else(|_| DEFAULT_LOCAL_DIR.to_owned());
    PathBuf::from(base).join(".thumbs")
}

#[derive(Debug, thiserror::Error)]
pub enum ThumbnailError {
    #[error("image decode failed")]
    Decode,
    #[error("image encode failed")]
    Encode,
    #[error("thumbnail cache io failed")]
    Io(#[from] std::io::Error),
}

#[derive(Debug)]
pub struct Thumbnail {
    pub bytes: Vec<u8>,
    pub mime: &'static str,
}

/// 命中磁盘缓存直接返回；否则缩放生成并落盘。`original` 的 MIME 必须是
/// [`is_resizable_image`] 认可的类型。目标宽不小于原图宽时原样返回，不做放大。
pub fn get_or_create(
    root: &Path,
    object_key: &str,
    original: &[u8],
    mime: &str,
    width: u32,
) -> Result<Thumbnail, ThumbnailError> {
    let cache_path = root.join(width.to_string()).join(object_key);
    if let Ok(bytes) = std::fs::read(&cache_path) {
        return Ok(Thumbnail {
            bytes,
            mime: output_mime(mime),
        });
    }

    let image = image::load_from_memory(original).map_err(|_| ThumbnailError::Decode)?;
    if image.width() <= width {
        // 不放大：直接返回原图，也不写缓存（原图本身就是最优响应）
        return Ok(Thumbnail {
            bytes: original.to_vec(),
            mime: output_mime(mime),
        });
    }
    let height = (u64::from(image.height()) * u64::from(width) / u64::from(image.width())) as u32;
    let resized = image::imageops::resize(
        &image,
        width,
        height.max(1),
        image::imageops::FilterType::Lanczos3,
    );

    let mut buffer = Vec::new();
    let format = match mime {
        "image/png" => ImageFormat::Png,
        "image/webp" => ImageFormat::WebP,
        // JPEG 不支持 Alpha，先转 RGB 再编码
        _ => {
            DynamicImage::ImageRgba8(resized)
                .into_rgb8()
                .write_to(&mut Cursor::new(&mut buffer), ImageFormat::Jpeg)
                .map_err(|_| ThumbnailError::Encode)?;
            write_cache(&cache_path, &buffer)?;
            return Ok(Thumbnail {
                bytes: buffer,
                mime: output_mime(mime),
            });
        }
    };
    DynamicImage::ImageRgba8(resized)
        .write_to(&mut Cursor::new(&mut buffer), format)
        .map_err(|_| ThumbnailError::Encode)?;
    write_cache(&cache_path, &buffer)?;
    Ok(Thumbnail {
        bytes: buffer,
        mime: output_mime(mime),
    })
}

fn output_mime(input: &str) -> &'static str {
    match input {
        "image/png" => "image/png",
        "image/webp" => "image/webp",
        _ => "image/jpeg",
    }
}

/// 临时文件 + rename 落盘：并发请求同时生成时，rename 是原子操作，
/// 失败方重试时自然命中已完成的缓存文件。
fn write_cache(cache_path: &Path, bytes: &[u8]) -> Result<(), ThumbnailError> {
    if let Some(parent) = cache_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temp_path = cache_path.with_extension("thumb-tmp");
    std::fs::write(&temp_path, bytes)?;
    match std::fs::rename(&temp_path, cache_path) {
        Ok(()) => Ok(()),
        // Windows 上目标已存在（并发完成）时 rename 失败，缓存内容等价，视为成功
        Err(error) if temp_path.exists() || cache_path.exists() => {
            let _ = std::fs::remove_file(&temp_path);
            if cache_path.exists() {
                Ok(())
            } else {
                Err(error.into())
            }
        }
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造一张纯色测试图并编码为 PNG。
    fn sample_png(width: u32, height: u32) -> Vec<u8> {
        let mut buffer = Vec::new();
        DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            width,
            height,
            image::Rgb([7, 8, 9]),
        ))
        .write_to(&mut Cursor::new(&mut buffer), ImageFormat::Png)
        .expect("encode png");
        buffer
    }

    fn temp_cache_root(name: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let root = std::env::temp_dir().join(format!(
            "aries-thumb-test-{name}-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).expect("create temp cache root");
        root
    }

    #[test]
    fn parse_width_accepts_only_numeric_in_range() {
        assert_eq!(parse_width(Some("480")), Some(480));
        assert_eq!(parse_width(Some("16")), Some(16));
        assert_eq!(parse_width(Some("1200")), Some(1200));
        assert_eq!(parse_width(Some("15")), None);
        assert_eq!(parse_width(Some("1201")), None);
        assert_eq!(parse_width(Some("abc")), None);
        assert_eq!(parse_width(Some("")), None);
        assert_eq!(parse_width(None), None);
    }

    #[test]
    fn resizable_mime_whitelist() {
        assert!(is_resizable_image("image/jpeg"));
        assert!(is_resizable_image("image/png"));
        assert!(is_resizable_image("image/webp"));
        assert!(!is_resizable_image("image/gif"));
        assert!(!is_resizable_image("image/svg+xml"));
    }

    #[test]
    fn get_or_create_resizes_and_caches_on_disk() {
        let root = temp_cache_root("resize");
        let original = sample_png(400, 200);
        let thumb = get_or_create(&root, "2026/10/sample.png", &original, "image/png", 100)
            .expect("thumbnail");

        let decoded = image::load_from_memory(&thumb.bytes).expect("decode thumbnail");
        assert_eq!(decoded.width(), 100);
        assert_eq!(decoded.height(), 50);
        assert!(thumb.bytes.len() < original.len());
        // 缓存文件已落盘，二次调用走缓存分支且内容一致
        let cache_file = root.join("100/2026/10/sample.png");
        assert!(cache_file.is_file());
        let again = get_or_create(&root, "2026/10/sample.png", &original, "image/png", 100)
            .expect("cached thumbnail");
        assert_eq!(again.bytes, thumb.bytes);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn get_or_create_never_enlarges() {
        let root = temp_cache_root("no-enlarge");
        let original = sample_png(100, 50);
        let thumb =
            get_or_create(&root, "sample.png", &original, "image/png", 400).expect("thumbnail");
        assert_eq!(thumb.bytes, original);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn get_or_create_rejects_undecodable_bytes() {
        let root = temp_cache_root("decode-error");
        let error =
            get_or_create(&root, "sample.png", b"not an image", "image/png", 100).unwrap_err();
        assert!(matches!(error, ThumbnailError::Decode));
        std::fs::remove_dir_all(&root).ok();
    }
}
