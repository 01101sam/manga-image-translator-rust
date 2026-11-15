use std::io;
use std::path::Path;
use std::sync::Arc;

use image::{codecs::jpeg::JpegEncoder, DynamicImage, RgbImage};
use tokio::sync::Semaphore;

use crate::server::source;

/// 允许的预览长边。限定一组值，缓存键才有上限，客户端也不能用任意 w 反复触发解码。
pub const WIDTHS: [u32; 4] = [256, 512, 1024, 2048];
/// 同时生成的预览数。每次生成都要把整张大图解码一遍，27 个并发请求不能同时来。
pub const MAX_CONCURRENT: usize = 2;
const JPEG_QUALITY: u8 = 82;

pub fn is_allowed(width: u32) -> bool {
    WIDTHS.contains(&width)
}

pub struct Previews {
    permits: Arc<Semaphore>,
}

impl Previews {
    pub fn new(limit: usize) -> Self {
        Self {
            permits: Arc::new(Semaphore::new(limit)),
        }
    }

    /// 缓存已存在就直接返回，否则排队拿到许可后在阻塞线程里生成。
    /// 排队期间别的请求可能已经生成了同一份，所以拿到许可后要再看一次。
    /// 许可跟着阻塞任务走：客户端断开会丢掉这个 future，但解码停不下来，许可不能提前还。
    pub async fn ensure<F>(&self, cache: &Path, render: F) -> io::Result<()>
    where
        F: FnOnce() -> io::Result<()> + Send + 'static,
    {
        if cache.exists() {
            return Ok(());
        }
        let permit = self
            .permits
            .clone()
            .acquire_owned()
            .await
            .map_err(io::Error::other)?;
        if cache.exists() {
            return Ok(());
        }
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            render()
        })
        .await
        .map_err(io::Error::other)?
    }
}

/// 解码源图，长边缩到 `width` 以内（不放大），存成 JPEG。先写临时文件再改名，读取方不会看到半个文件。
pub fn render(source_path: &Path, cache: &Path, width: u32) -> io::Result<()> {
    let img = source::decode(source_path).map_err(io::Error::other)?;
    let img = if img.width().max(img.height()) > width {
        img.thumbnail(width, width)
    } else {
        img
    };
    let rgb = flatten_on_white(img);
    let tmp = cache.with_extension(format!("{}.tmp", uuid::Uuid::new_v4().simple()));
    let result = std::fs::File::create(&tmp)
        .map(io::BufWriter::new)
        .and_then(|mut out| {
            JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY)
                .encode_image(&rgb)
                .map_err(io::Error::other)
        })
        .and_then(|()| std::fs::rename(&tmp, cache));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// JPEG 没有 alpha 通道，透明处铺白底而不是留下底层的黑色。
fn flatten_on_white(img: DynamicImage) -> RgbImage {
    if !img.color().has_alpha() {
        return img.into_rgb8();
    }
    let rgba = img.into_rgba8();
    let mut out = RgbImage::new(rgba.width(), rgba.height());
    for (src, dst) in rgba.pixels().zip(out.pixels_mut()) {
        let alpha = u32::from(src[3]);
        for c in 0..3 {
            dst[c] = ((u32::from(src[c]) * alpha + 255 * (255 - alpha) + 127) / 255) as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use image::{ImageFormat, Rgba, RgbaImage};

    use super::*;
    use crate::server::source::test_support::png_bytes;

    fn tmp_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("it-prev-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn at_most_the_limit_render_at_once_and_all_of_them_finish() {
        let dir = tmp_dir();
        let previews = Arc::new(Previews::new(MAX_CONCURRENT));
        let running = Arc::new(AtomicUsize::new(0));
        let high_water = Arc::new(AtomicUsize::new(0));
        let mut tasks = Vec::new();
        for i in 0..8 {
            let (previews, running, high_water) =
                (previews.clone(), running.clone(), high_water.clone());
            let cache = dir.join(format!("{i}.jpg"));
            tasks.push(tokio::spawn(async move {
                let target = cache.clone();
                previews
                    .ensure(&cache, move || {
                        let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                        high_water.fetch_max(now, Ordering::SeqCst);
                        std::thread::sleep(Duration::from_millis(60));
                        running.fetch_sub(1, Ordering::SeqCst);
                        std::fs::write(target, b"jpeg")
                    })
                    .await
            }));
        }
        for t in tasks {
            t.await.unwrap().unwrap();
        }
        assert_eq!(high_water.load(Ordering::SeqCst), MAX_CONCURRENT);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 8);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn the_limit_still_holds_when_clients_cancel_mid_render() {
        let dir = tmp_dir();
        let previews = Arc::new(Previews::new(MAX_CONCURRENT));
        let running = Arc::new(AtomicUsize::new(0));
        let high_water = Arc::new(AtomicUsize::new(0));
        let spawn_one = |i: usize| {
            let (previews, running, high_water) =
                (previews.clone(), running.clone(), high_water.clone());
            let cache = dir.join(format!("{i}.jpg"));
            tokio::spawn(async move {
                let target = cache.clone();
                let _ = previews
                    .ensure(&cache, move || {
                        let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                        high_water.fetch_max(now, Ordering::SeqCst);
                        std::thread::sleep(Duration::from_millis(80));
                        running.fetch_sub(1, Ordering::SeqCst);
                        std::fs::write(target, b"jpeg")
                    })
                    .await;
            })
        };
        let first_wave: Vec<_> = (0..8).map(spawn_one).collect();
        tokio::time::sleep(Duration::from_millis(20)).await;
        // 客户端断开会丢掉 handler 的 future，但阻塞线程里的解码停不下来。
        for task in &first_wave {
            task.abort();
        }
        let second_wave: Vec<_> = (8..12).map(spawn_one).collect();
        for task in second_wave {
            task.await.unwrap();
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(running.load(Ordering::SeqCst), 0, "every render finished");
        assert!(high_water.load(Ordering::SeqCst) <= MAX_CONCURRENT);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn an_existing_cache_skips_rendering() {
        let dir = tmp_dir();
        let cache = dir.join("hit.jpg");
        std::fs::write(&cache, b"cached").unwrap();
        Previews::new(1)
            .ensure(&cache, || panic!("must not render on a cache hit"))
            .await
            .unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn render_downsizes_without_upscaling_and_writes_a_jpeg() {
        let dir = tmp_dir();
        let src = dir.join("src");
        std::fs::write(&src, png_bytes(1200, 800)).unwrap();
        let big = dir.join("big.jpg");
        render(&src, &big, 256).unwrap();
        let small = image::open(&big).unwrap();
        assert_eq!((small.width(), small.height()), (256, 171));
        assert_eq!(
            image::guess_format(&std::fs::read(&big).unwrap()).unwrap(),
            ImageFormat::Jpeg
        );
        let same = dir.join("same.jpg");
        render(&src, &same, 2048).unwrap();
        let kept = image::open(&same).unwrap();
        assert_eq!((kept.width(), kept.height()), (1200, 800));
        assert_eq!(
            std::fs::read_dir(&dir).unwrap().count(),
            3,
            "no temp files left"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn transparent_pixels_become_white() {
        let dir = tmp_dir();
        let src = dir.join("src");
        let mut out = std::io::Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(RgbaImage::from_pixel(8, 8, Rgba([0, 0, 0, 0])))
            .write_to(&mut out, ImageFormat::Png)
            .unwrap();
        std::fs::write(&src, out.into_inner()).unwrap();
        let cache = dir.join("t.jpg");
        render(&src, &cache, 256).unwrap();
        let px = image::open(&cache).unwrap().to_rgb8().get_pixel(4, 4).0;
        assert!(px.iter().all(|&c| c >= 250), "{px:?}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_failed_render_leaves_no_cache_and_no_temp_file() {
        let dir = tmp_dir();
        let src = dir.join("src");
        std::fs::write(&src, b"not an image").unwrap();
        let cache = dir.join("x.jpg");
        assert!(render(&src, &cache, 256).is_err());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
