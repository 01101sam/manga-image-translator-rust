use std::sync::Arc;
use std::time::Instant;

use base_util::onnx::all_providers;
use interface_image::{CpuImageProcessor, ImageOp, Mask, RawImage};
use interface_inpainter::{Inpainter, InpainterOptions};
use lama_aot::LamaLargeInpainter;
use ndarray::Array2;

const WARMUP: usize = 1;
const ITERS: usize = 5;

fn fnv(bytes: impl IntoIterator<Item = u8>) -> u64 {
    bytes
        .into_iter()
        .fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3))
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

/// Usage (from repo root): bench_inpaint [all|cpu] [image path] [mask .npy path]
///
/// `RUST_LOG=info` shows the execution provider in use. `BENCH_DUMP=<path>` writes the raw output
/// pixels of the last iteration for a cross-provider diff.
#[tokio::main]
async fn main() {
    env_logger::init();
    let mut args = std::env::args().skip(1);
    let providers = args.next().unwrap_or_else(|| "all".into());
    let img_path = args
        .next()
        .unwrap_or_else(|| "imgs/232265329-6a560438-e887-4f7f-b6a1-a61b8648f781.png".into());
    let mask_path = args
        .next()
        .unwrap_or_else(|| "crates/modules/inpainter/lama_large/mask.npy".into());
    let providers = Arc::new(match providers.as_str() {
        "all" => all_providers(),
        "cpu" => vec![],
        _ => panic!("unknown provider set {providers}"),
    });
    let inp = LamaLargeInpainter::new(providers);
    let img = RawImage::new(&img_path).expect("image");
    let mask: Array2<u8> = ndarray_npy::read_npy(&mask_path).expect("mask");
    let mask = Mask::from(mask);
    let ip = Arc::new(CpuImageProcessor::default()) as Arc<dyn ImageOp + Send + Sync>;

    let mut times = Vec::new();
    let mut fingerprint = String::new();
    for i in 0..WARMUP + ITERS {
        let t0 = Instant::now();
        let out = inp
            .inpaint(&img, mask.clone(), InpainterOptions::default(), &ip)
            .await
            .expect("inpaint");
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        fingerprint = format!(
            "out={}x{} fnv={:016x}",
            out.width,
            out.height,
            fnv(out.data.iter().copied())
        );
        if i >= WARMUP {
            times.push(ms);
        }
        if let Ok(dump) = std::env::var("BENCH_DUMP") {
            std::fs::write(&dump, &out.data).expect("dump image");
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
        "bench_inpaint img={} {}x{} iters={} min_ms={:.1} median_ms={:.1} mean_ms={:.1} {}",
        img_path,
        img.width,
        img.height,
        ITERS,
        min,
        median(times),
        mean,
        fingerprint
    );
}
