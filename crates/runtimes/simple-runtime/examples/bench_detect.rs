use std::sync::Arc;
use std::time::Instant;

use base_util::onnx::all_providers;
use interface_detector::{DefaultOptions, Detector, PreprocessorOptions};
use interface_image::{CpuImageProcessor, ImageOp, RawImage};

const WARMUP: usize = 2;
const ITERS: usize = 10;

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

/// Usage: bench_detect [dbnet|ctd] [image path relative to repo root]
#[tokio::main]
async fn main() {
    env_logger::init();
    let mut args = std::env::args().skip(1);
    let which = args.next().unwrap_or_else(|| "dbnet".into());
    let path = args
        .next()
        .unwrap_or_else(|| "imgs/232264684-5a7bcf8e-707b-4925-86b0-4212382f1680.png".into());
    let providers = Arc::new(all_providers());
    let det: Box<dyn Detector + Send + Sync> = match which.as_str() {
        "dbnet" => Box::new(dbnet::DbNetDetector::new(providers, false)),
        "ctd" => Box::new(ctd::CtdDetector::new(providers)),
        _ => panic!("unknown detector {which}"),
    };
    let img = RawImage::new(&path).expect("image");
    let ip = Arc::new(CpuImageProcessor::default()) as Arc<dyn ImageOp + Send + Sync>;

    let mut times = Vec::new();
    let mut fingerprint = String::new();
    for i in 0..WARMUP + ITERS {
        let t0 = Instant::now();
        let (boxes, mask) = det
            .detect(
                &img,
                PreprocessorOptions::default(),
                DefaultOptions::default(),
                &ip,
            )
            .await
            .expect("detect");
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        let mut coords: Vec<i64> = boxes
            .iter()
            .flat_map(|q| q.pts().iter().flat_map(|p| [p.x, p.y]))
            .collect();
        coords.sort_unstable();
        let box_fnv = fnv(coords.iter().flat_map(|c| c.to_le_bytes()));
        let mask_nz = mask.data.iter().filter(|&&v| v > 0).count();
        fingerprint = format!(
            "boxes={} box_fnv={:016x} mask={}x{} mask_nz={} mask_fnv={:016x}",
            boxes.len(),
            box_fnv,
            mask.width,
            mask.height,
            mask_nz,
            fnv(mask.data.iter().copied())
        );
        if i >= WARMUP {
            times.push(ms);
        }
        if let Ok(dump) = std::env::var("BENCH_DUMP") {
            std::fs::write(&dump, &mask.data).expect("dump mask");
            let mut quads: Vec<String> = boxes
                .iter()
                .map(|q| {
                    format!(
                        "{:?}",
                        q.pts().iter().map(|p| (p.x, p.y)).collect::<Vec<_>>()
                    )
                })
                .collect();
            quads.sort();
            std::fs::write(format!("{dump}.boxes"), quads.join("\n")).expect("dump boxes");
        }
        eprintln!(
            "iter={} {:.1}ms {}{}",
            i,
            ms,
            fingerprint,
            if i < WARMUP { " warmup" } else { "" }
        );
    }
    let min = times.iter().copied().fold(f64::INFINITY, f64::min);
    let mean = times.iter().sum::<f64>() / times.len() as f64;
    println!(
        "bench_detect det={} img={} {}x{} iters={} min_ms={:.1} median_ms={:.1} mean_ms={:.1} {}",
        which,
        path,
        img.width,
        img.height,
        ITERS,
        min,
        median(times),
        mean,
        fingerprint
    );
}
