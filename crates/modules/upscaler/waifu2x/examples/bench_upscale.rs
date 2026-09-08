use std::sync::Arc;
use std::time::Instant;

use base_util::onnx::all_providers;
use interface_image::{CpuImageProcessor, ImageOp, RawImage};
use interface_upscaler::Upscaler;
use util::lama::resize_keep_aspect;
use waifu2x::{Waifu2xModels, Waifu2xUpscaler};

const WARMUP: usize = 1;
const ITERS: usize = 3;
/// Default longest side of the bench input; the 1487x2048 page is too large to upscale whole.
const INPUT_SIZE: u16 = 512;

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

/// Usage (from repo root): bench_upscale [all|cpu] [cunet|swin2x|swin4x] [image path] [longest side]
///
/// `RUST_LOG=info` shows the execution provider in use. `BENCH_DUMP=<path>` writes the raw output
/// pixels of the last iteration for a cross-provider diff.
#[tokio::main]
async fn main() {
    env_logger::init();
    let mut args = std::env::args().skip(1);
    let providers = args.next().unwrap_or_else(|| "all".into());
    let variant = args.next().unwrap_or_else(|| "cunet".into());
    let img_path = args
        .next()
        .unwrap_or_else(|| "imgs/232265329-6a560438-e887-4f7f-b6a1-a61b8648f781.png".into());
    let input_size: u16 = args
        .next()
        .map(|s| s.parse().expect("longest side"))
        .unwrap_or(INPUT_SIZE);
    let providers = Arc::new(match providers.as_str() {
        "all" => all_providers(),
        "cpu" => vec![],
        _ => panic!("unknown provider set {providers}"),
    });
    let model = match variant.as_str() {
        "cunet" => Waifu2xModels::CuNetArt { noise: Some(3) },
        "swin2x" => Waifu2xModels::SwinUnetArt {
            x4: false,
            noise: Some(3),
        },
        "swin4x" => Waifu2xModels::SwinUnetArt {
            x4: true,
            noise: Some(3),
        },
        _ => panic!("unknown variant {variant}"),
    };
    let up = Waifu2xUpscaler::new(model, 5, providers);
    let ip = Arc::new(CpuImageProcessor::default()) as Arc<dyn ImageOp + Send + Sync>;
    let img = RawImage::new(&img_path).expect("image");
    let img = resize_keep_aspect(img.view(), input_size, &ip).expect("resize");

    let mut times = Vec::new();
    let mut fingerprint = String::new();
    for i in 0..WARMUP + ITERS {
        let t0 = Instant::now();
        let out = up.upscale(&img, None, 0, &ip).await.expect("upscale");
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
        "bench_upscale model={} img={} {}x{} iters={} min_ms={:.1} median_ms={:.1} mean_ms={:.1} {}",
        variant,
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
