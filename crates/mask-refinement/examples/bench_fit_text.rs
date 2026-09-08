use std::sync::Arc;
use std::time::Instant;

use interface_detector::textlines::MyPoint;
use interface_image::{CpuImageProcessor, ImageOp, Mask, RawImage};
use interface_translator::LangIdDetector;
use mask_refinement::Method;
use textline_merge::TextBlock;

const W: u16 = 1200;
const H: u16 = 1800;
const BOXES: &[(i64, i64, i64, i64)] = &[
    (80, 100, 220, 90),
    (340, 110, 240, 90),
    (80, 420, 280, 110),
    (700, 200, 380, 120),
    (90, 800, 330, 120),
    (500, 780, 400, 120),
    (120, 1200, 380, 160),
    (620, 1300, 480, 180),
];
const WARMUP: usize = 2;
const ITERS: usize = 8;

fn fixture() -> (RawImage, Mask, Vec<TextBlock>) {
    let mut img = vec![245u8; W as usize * H as usize * 3];
    let mut mask = vec![0u8; W as usize * H as usize];
    let det = LangIdDetector::new().expect("langid");
    let mut blocks = Vec::new();
    for &(x, y, w, h) in BOXES {
        let x1 = x.max(0) as usize;
        let y1 = y.max(0) as usize;
        let x2 = ((x + w) as usize).min(W as usize);
        let y2 = ((y + h) as usize).min(H as usize);
        for yy in y1..y2 {
            if (yy - y1) % 4 == 0 {
                continue;
            }
            for xx in x1..x2 {
                let i = (yy * W as usize + xx) * 3;
                img[i] = 20;
                img[i + 1] = 20;
                img[i + 2] = 20;
            }
        }
        let mx1 = x.saturating_sub(6) as usize;
        let my1 = y.saturating_sub(6) as usize;
        let mx2 = ((x + w + 6) as usize).min(W as usize);
        let my2 = ((y + h + 6) as usize).min(H as usize);
        for yy in my1..my2 {
            for xx in mx1..mx2 {
                mask[yy * W as usize + xx] = 255;
            }
        }
        blocks.push(TextBlock::new(
            vec![[
                MyPoint { x, y },
                MyPoint { x: x + w, y },
                MyPoint { x: x + w, y: y + h },
                MyPoint { x, y: y + h },
            ]],
            vec![String::new()],
            h as u64,
            0.0,
            1.0,
            None,
            None,
            &det,
        ));
    }
    (
        RawImage {
            data: img,
            width: W,
            height: H,
            channels: 3,
        },
        Mask {
            data: mask,
            width: W,
            height: H,
        },
        blocks,
    )
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

fn main() {
    let ip = Arc::new(CpuImageProcessor::default()) as Arc<dyn ImageOp + Send + Sync>;
    let (img, mask, blocks) = fixture();
    let mut times = Vec::new();
    let mut last_nz = 0usize;
    let mut last_fnv = 0u64;
    for i in 0..WARMUP + ITERS {
        let t0 = Instant::now();
        let out = mask_refinement::dispatch(
            &blocks,
            &img,
            &mask,
            Method::FitText,
            0,
            20.0,
            3,
            false,
            &ip,
        )
        .expect("dispatch");
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        last_nz = out.data.iter().filter(|&&v| v > 0).count();
        last_fnv = out
            .data
            .iter()
            .fold(0xcbf29ce484222325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x100000001b3));
        if i >= WARMUP {
            times.push(ms);
        }
        eprintln!(
            "rust iter={} {:.1}ms nz={}{}",
            i,
            ms,
            last_nz,
            if i < WARMUP { " warmup" } else { "" }
        );
    }
    let min = times.iter().copied().fold(f64::INFINITY, f64::min);
    let mean = times.iter().sum::<f64>() / times.len() as f64;
    println!(
        "rust_fit_text w={} h={} boxes={} warmup={} iters={} min_ms={:.1} median_ms={:.1} mean_ms={:.1} nz={} fnv={:016x}",
        W,
        H,
        BOXES.len(),
        WARMUP,
        ITERS,
        min,
        median(times),
        mean,
        last_nz,
        last_fnv
    );
}
