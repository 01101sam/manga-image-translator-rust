use std::time::Instant;

use export::Export;
use image::{DynamicImage, Rgba, RgbaImage};
use interface_detector::textlines::MyPoint;
use interface_translator::LangIdDetector;
use png::{PngRenderConfig, PngRenderer};
use textline_merge::TextBlock;

const WARMUP: usize = 2;
const ITERS: usize = 10;

/// One speech bubble: (x, y, line count, line length px, font px, vertical, angle deg).
/// Laid out for a 1487x2048 manga page, mostly vertical Japanese lines; two are tilted so the
/// warp path sees rotated destination quads too.
const BUBBLES: &[(i64, i64, usize, i64, i64, bool, f64)] = &[
    (1180, 90, 4, 420, 34, true, 0.0),
    (760, 120, 3, 300, 30, true, 0.0),
    (240, 160, 5, 380, 32, true, 8.0),
    (1240, 640, 2, 260, 36, true, 0.0),
    (820, 700, 4, 340, 30, true, 0.0),
    (360, 760, 3, 240, 28, true, 0.0),
    (110, 1100, 4, 300, 30, true, 0.0),
    (1150, 1180, 5, 460, 34, true, -12.0),
    (700, 1260, 2, 200, 26, true, 0.0),
    (1050, 1700, 3, 280, 32, true, 0.0),
    (300, 1720, 4, 240, 30, true, 0.0),
    (80, 40, 3, 380, 24, false, 0.0),
    (500, 1980, 2, 420, 22, false, 0.0),
    // 故意超长译文压测块：一横（窄框多行）一竖（多列）。
    (900, 1900, 2, 200, 24, false, 0.0),
    (1350, 850, 2, 150, 26, true, 0.0),
];

const TRANSLATIONS: &[&str] = &[
    "才没有那种事呢",
    "你真的这么想吗?",
    "走吧,没时间了",
    "等一下!",
    "到底发生了什么",
    "没事的,别担心",
    "That is not what I meant at all",
];

/// 故意超长的压测译文（无空格，专测换行/换列与缩小不丢字）。
const LONG_H: &str = "这是一个故意写得非常非常长的中文译文句子用来测试横排换行与字号缩小会不会丢字";
const LONG_V: &str = "竖排超长译文列打包测试看看列数增长与字号缩小能不能装下全部的字形不丢失";

fn blocks(det: &LangIdDetector) -> Vec<TextBlock> {
    BUBBLES
        .iter()
        .enumerate()
        .map(|(b, &(x, y, n, len, font, vertical, angle))| {
            let pitch = font * 13 / 10;
            let lines = (0..n as i64)
                .map(|i| {
                    if vertical {
                        let cx = x - i * pitch;
                        [(cx, y), (cx + font, y), (cx + font, y + len), (cx, y + len)]
                    } else {
                        let cy = y + i * pitch;
                        [(x, cy), (x + len, cy), (x + len, cy + font), (x, cy + font)]
                    }
                    .map(|(x, y)| MyPoint { x, y })
                })
                .collect();
            TextBlock::new(
                lines,
                vec!["src".to_owned(); n],
                font as u64,
                angle,
                1.0,
                Some((20, 20, 20)),
                Some((250, 250, 250)),
                det,
            )
            .with_translation("CHS", match b {
                13 => LONG_H,
                14 => LONG_V,
                _ => TRANSLATIONS[b % TRANSLATIONS.len()],
            })
        })
        .collect()
}

/// Inpainted overlay: opaque white over every bubble's bounding box, transparent elsewhere.
fn overlay(w: u32, h: u32, blocks: &[TextBlock]) -> RgbaImage {
    let mut img = RgbaImage::new(w, h);
    for b in blocks {
        let (x1, y1, x2, y2) = b.xyxy();
        for y in (y1 - 10).max(0)..(y2 + 10).min(h as i64) {
            for x in (x1 - 10).max(0)..(x2 + 10).min(w as i64) {
                img.put_pixel(x as u32, y as u32, Rgba([250, 250, 250, 255]));
            }
        }
    }
    img
}

fn fnv(bytes: impl IntoIterator<Item = u8>) -> u64 {
    bytes.into_iter().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100000001b3)
    })
}

fn median(mut xs: Vec<f64>) -> f64 {
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = xs.len();
    if n % 2 == 0 {
        (xs[n / 2 - 1] + xs[n / 2]) / 2.0
    } else {
        xs[n / 2]
    }
}

fn report(name: &str, times: &[f64], extra: String) {
    let min = times.iter().copied().fold(f64::INFINITY, f64::min);
    let mean = times.iter().sum::<f64>() / times.len() as f64;
    println!(
        "bench_render {name} iters={} min_ms={min:.1} median_ms={:.1} mean_ms={mean:.1} {extra}",
        times.len(),
        median(times.to_vec()),
    );
}

/// Usage: bench_render [image path relative to repo root]
/// BENCH_DUMP=<path.png> writes the last rendered page.
fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "imgs/232265329-6a560438-e887-4f7f-b6a1-a61b8648f781.png".into());
    let page = image::open(&path).expect("image");
    let det = LangIdDetector::new().unwrap();
    let overlay = DynamicImage::ImageRgba8(overlay(page.width(), page.height(), &blocks(&det)));
    eprintln!(
        "blocks={} img={}x{}",
        BUBBLES.len(),
        page.width(),
        page.height()
    );

    let mut init_times = Vec::new();
    for i in 0..WARMUP + ITERS {
        let t0 = Instant::now();
        let renderer = PngRenderer::default();
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        std::hint::black_box(renderer);
        if i >= WARMUP {
            init_times.push(ms);
        }
    }
    report("PngRenderer::default", &init_times, String::new());

    let mut renderer = PngRenderer::default();
    let mut times = Vec::new();
    let mut encode_times = Vec::new();
    let mut fingerprint = String::new();
    for i in 0..WARMUP + ITERS {
        let exp = Export::new(page.clone(), overlay.clone(), blocks(&det), None);
        let t0 = Instant::now();
        let out = renderer
            .render(exp, PngRenderConfig::default())
            .expect("render");
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        // 每轮排空报告（防无限增长），并断言无缺失；只在最后一轮打印。
        let reports = renderer.take_reports();
        for r in &reports {
            assert_eq!(
                r.glyphs_out, r.glyphs_in,
                "block {} lost glyphs: {r}",
                r.index
            );
        }
        if i == WARMUP + ITERS - 1 {
            for r in &reports {
                println!("{r}");
            }
        }
        fingerprint = format!(
            "out={}x{}x{} fnv={:016x}",
            out.width,
            out.height,
            out.channels,
            fnv(out.data.iter().copied())
        );
        if i >= WARMUP {
            times.push(ms);
        }
        eprintln!(
            "iter={i} {ms:.1}ms {fingerprint}{}",
            if i < WARMUP { " warmup" } else { "" }
        );
        // The CLI's "PERF render" also includes PNG encoding of the page; timed separately for context.
        let t0 = Instant::now();
        let mut encoded = std::io::Cursor::new(Vec::new());
        let img = out.to_image().expect("to_image");
        img.write_to(&mut encoded, image::ImageFormat::Png)
            .expect("encode");
        if i >= WARMUP {
            encode_times.push(t0.elapsed().as_secs_f64() * 1000.0);
        }
        if let Ok(dump) = std::env::var("BENCH_DUMP") {
            std::fs::write(dump, encoded.into_inner()).expect("dump");
        }
    }
    report("render", &times, fingerprint);
    report("png_encode", &encode_times, String::new());
}
