use std::path::Path;

use image::{imageops::FilterType, DynamicImage, ImageFormat, ImageReader, Limits};

/// 解码前按文件头拒绝超大图。一张 200MP 的 RGBA 图解码后就是 800MB。
pub const MAX_PIXELS: u64 = 200_000_000;
/// RawImage 的边长类型是 u16，更长的边在转换时会被静默截断。
pub const MAX_EDGE: u32 = u16::MAX as u32;
/// image 默认的 512MiB 解码上限低于 200MP 的 RGB 图，不放宽的话合法大图会在 Worker 里不明原因地失败。
const DECODE_ALLOC_LIMIT: u64 = 2 << 30;

#[derive(Debug, PartialEq, Eq)]
pub enum ProbeError {
    NotAnImage(String),
    TooLarge { width: u32, height: u32 },
}

impl ProbeError {
    pub fn message(&self) -> String {
        match self {
            ProbeError::NotAnImage(e) => format!("无法识别图片: {e}"),
            ProbeError::TooLarge { width, height } => format!(
                "图片 {width}x{height} 太大, 上限是 {}MP 且单边不超过 {MAX_EDGE} 像素",
                MAX_PIXELS / 1_000_000
            ),
        }
    }
}

/// 只读文件头：返回源图的 MIME 并检查尺寸，不分配像素缓冲。
pub fn probe(path: &Path) -> Result<&'static str, ProbeError> {
    let not_image = |e: &dyn std::fmt::Display| ProbeError::NotAnImage(e.to_string());
    let reader = ImageReader::open(path)
        .and_then(|r| r.with_guessed_format())
        .map_err(|e| not_image(&e))?;
    let format = reader
        .format()
        .ok_or_else(|| not_image(&"不是支持的图片格式"))?;
    let (width, height) = reader.into_dimensions().map_err(|e| not_image(&e))?;
    if u64::from(width) * u64::from(height) > MAX_PIXELS || width.max(height) > MAX_EDGE {
        return Err(ProbeError::TooLarge { width, height });
    }
    Ok(mime_of_format(format))
}

fn mime_of_format(format: ImageFormat) -> &'static str {
    format.to_mime_type()
}

/// 整张解码，放宽 image 默认的内存上限。要在阻塞线程里调用。
pub fn decode(path: &Path) -> anyhow::Result<DynamicImage> {
    let mut reader = ImageReader::open(path)?.with_guessed_format()?;
    let mut limits = Limits::default();
    limits.max_alloc = Some(DECODE_ALLOC_LIMIT);
    reader.limits(limits);
    Ok(reader.decode()?)
}

/// 在阻塞线程里调用。长边超过 `long_edge` 时等比缩小，之后整条 pipeline 只处理缩小后的图。
pub fn decode_for_pipeline(path: &Path, long_edge: Option<u32>) -> anyhow::Result<DynamicImage> {
    let img = decode(path)?;
    Ok(match long_edge {
        Some(edge) if img.width().max(img.height()) > edge => {
            img.resize(edge, edge, FilterType::Triangle)
        }
        _ => img,
    })
}

#[cfg(test)]
pub mod test_support {
    use std::io::Cursor;

    use image::{DynamicImage, ImageFormat};

    pub fn png_bytes(width: u32, height: u32) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        DynamicImage::new_rgb8(width, height)
            .write_to(&mut out, ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }

    /// 只改 IHDR 里的尺寸并重算 CRC：能通过文件头检查，又不用真的编码几百 MP 的像素。
    pub fn png_header_claiming(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = png_bytes(1, 1);
        bytes[16..20].copy_from_slice(&width.to_be_bytes());
        bytes[20..24].copy_from_slice(&height.to_be_bytes());
        let crc = crc32(&bytes[12..29]);
        bytes[29..33].copy_from_slice(&crc.to_be_bytes());
        bytes
    }

    fn crc32(data: &[u8]) -> u32 {
        let mut crc = 0xFFFF_FFFFu32;
        for &b in data {
            crc ^= u32::from(b);
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{png_bytes, png_header_claiming};
    use super::*;

    fn write_tmp(bytes: &[u8]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("it-src-{}", uuid::Uuid::new_v4()));
        std::fs::write(&path, bytes).unwrap();
        path
    }
    #[test]
    fn probe_reads_mime_without_decoding() {
        let path = write_tmp(&png_bytes(8, 4));
        assert_eq!(probe(&path), Ok("image/png"));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn probe_rejects_more_than_200mp_from_the_header_alone() {
        let path = write_tmp(&png_header_claiming(20_000, 20_000));
        assert_eq!(
            probe(&path),
            Err(ProbeError::TooLarge {
                width: 20_000,
                height: 20_000
            })
        );
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn probe_accepts_exactly_200mp() {
        let path = write_tmp(&png_header_claiming(20_000, 10_000));
        assert_eq!(probe(&path), Ok("image/png"));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn probe_rejects_an_edge_beyond_u16_even_under_the_pixel_cap() {
        let path = write_tmp(&png_header_claiming(800, 100_000));
        assert!(matches!(probe(&path), Err(ProbeError::TooLarge { .. })));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn probe_rejects_non_images() {
        let path = write_tmp(b"definitely not an image");
        assert!(matches!(probe(&path), Err(ProbeError::NotAnImage(_))));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn decode_downscales_the_long_edge_and_keeps_the_aspect_ratio() {
        let path = write_tmp(&png_bytes(5000, 2000));
        let img = decode_for_pipeline(&path, Some(4096)).unwrap();
        assert_eq!((img.width(), img.height()), (4096, 1638));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn decode_leaves_small_images_and_the_disabled_setting_alone() {
        let path = write_tmp(&png_bytes(5000, 2000));
        let small = decode_for_pipeline(&path, Some(8000)).unwrap();
        assert_eq!((small.width(), small.height()), (5000, 2000));
        let off = decode_for_pipeline(&path, None).unwrap();
        assert_eq!((off.width(), off.height()), (5000, 2000));
        std::fs::remove_file(path).unwrap();
    }
}
