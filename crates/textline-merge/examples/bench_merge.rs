use std::sync::Arc;
use std::time::Instant;

use interface_detector::textlines::Quadrilateral;
use interface_image::RawImage;
use interface_ocr::QuadrilateralInfo;
use interface_translator::LangIdDetector;
use textline_merge::TextBlock;

const WARMUP: usize = 2;
const ITERS: usize = 10;

/// One speech bubble: (x, y, line count, line length px, font px, vertical).
/// Laid out for a 1487x2048 manga page, mostly vertical Japanese lines.
const BUBBLES: &[(i64, i64, usize, i64, i64, bool)] = &[
    (1180, 90, 4, 420, 34, true),
    (760, 120, 3, 300, 30, true),
    (240, 160, 5, 380, 32, true),
    (1240, 640, 2, 260, 36, true),
    (820, 700, 4, 340, 30, true),
    (360, 760, 3, 240, 28, true),
    (110, 1100, 4, 300, 30, true),
    (1150, 1180, 5, 460, 34, true),
    (700, 1260, 2, 200, 26, true),
    (1050, 1700, 3, 280, 32, true),
    (300, 1720, 4, 240, 30, true),
    (80, 40, 3, 380, 24, false),
    (500, 1980, 2, 420, 22, false),
];

const TEXTS: &[&str] = &[
    "そんなことはないよ",
    "本当にそう思うのか",
    "行こう、時間がない",
    "待ってくれ！",
    "何が起きているんだ",
    "大丈夫、心配しないで",
];

fn textlines() -> Vec<QuadrilateralInfo> {
    let mut out = Vec::new();
    for (b, &(x, y, n, len, font, vertical)) in BUBBLES.iter().enumerate() {
        for i in 0..n {
            let jitter = ((b * 7 + i * 3) % 5) as i64 - 2;
            let pitch = font * 13 / 10;
            let pts = if vertical {
                let cx = x - (i as i64) * pitch;
                vec![
                    (cx, y + jitter),
                    (cx + font, y),
                    (cx + font, y + len + jitter),
                    (cx, y + len),
                ]
            } else {
                let cy = y + (i as i64) * pitch;
                vec![
                    (x + jitter, cy),
                    (x + len, cy + jitter),
                    (x + len, cy + font),
                    (x, cy + font),
                ]
            };
            out.push(QuadrilateralInfo {
                text: TEXTS[(b + i) % TEXTS.len()].to_owned(),
                fg: Some([20, 20, 20]),
                bg: Some([250, 250, 250]),
                pos: Arc::new(Quadrilateral::new(pts, 0.95).into()),
                prob: 0.95,
            });
        }
    }
    out
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

fn fingerprint(blocks: &[TextBlock]) -> u64 {
    fnv(blocks.iter().flat_map(|b| {
        let (x1, y1, x2, y2) = b.xyxy();
        [x1, y1, x2, y2, b.lines.len() as i64, b.font_size as i64]
            .into_iter()
            .flat_map(|v| v.to_le_bytes())
            .chain(b.text.bytes())
            .collect::<Vec<_>>()
    }))
}

/// `f` returns the timed section's duration in ms (so untimed setup can run inside it), and the result fingerprint.
fn bench(name: &str, mut f: impl FnMut() -> (f64, (u64, usize))) {
    let mut times = Vec::new();
    let mut fp = (0, 0);
    for i in 0..WARMUP + ITERS {
        let (ms, out) = f();
        fp = out;
        if i >= WARMUP {
            times.push(ms);
        }
    }
    let min = times.iter().copied().fold(f64::INFINITY, f64::min);
    let mean = times.iter().sum::<f64>() / times.len() as f64;
    println!(
        "bench_merge {name} iters={ITERS} min_ms={min:.2} median_ms={:.2} mean_ms={mean:.2} blocks={} fnv={:016x}",
        median(times),
        fp.1,
        fp.0
    );
}

/// Usage: bench_merge [image path relative to repo root]
fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "imgs/232265329-6a560438-e887-4f7f-b6a1-a61b8648f781.png".into());
    let img = RawImage::new(&path).expect("image");
    let lines = textlines();
    let det = LangIdDetector::new().unwrap();
    let remove_text = Vec::new();
    let merge = || {
        textline_merge::dispatch_main(
            &lines,
            img.width,
            img.height,
            1,
            0.2,
            vec![],
            &remove_text,
            &det,
        )
        .expect("merge")
    };
    eprintln!("textlines={} img={}x{}", lines.len(), img.width, img.height);

    let result = |t0: Instant, out: Vec<TextBlock>| {
        (
            t0.elapsed().as_secs_f64() * 1000.0,
            (fingerprint(&out), out.len()),
        )
    };

    bench("dispatch_main", || {
        let t0 = Instant::now();
        result(t0, merge())
    });
    bench("sort_regions(panels)", || {
        let blocks = merge();
        let t0 = Instant::now();
        result(
            t0,
            textline_merge::sort_regions(blocks, true, Some(&img), false),
        )
    });
    bench("sort_regions(simple)", || {
        let blocks = merge();
        let t0 = Instant::now();
        result(t0, textline_merge::sort_regions(blocks, true, None, true))
    });
}
