use std::sync::Arc;
use std::time::Instant;

use base_util::onnx::all_providers;
use interface_image::{CpuImageProcessor, ImageOp, RawImage};
use pixai_tagger::{PixaiTagger, TaggerOptions};

const WARMUP: usize = 2;
const ITERS: usize = 10;

fn median(mut xs: Vec<f64>) -> f64 {
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = xs.len();
    if n % 2 == 0 {
        (xs[n / 2 - 1] + xs[n / 2]) / 2.0
    } else {
        xs[n / 2]
    }
}

/// Usage (from repo root): bench_tag [all|cpu] [image path]
///
/// `RUST_LOG=info` shows the execution provider in use; `ORT_LOG=verbose RUST_LOG=trace` adds
/// ORT's own partitioning logs (which nodes CoreML accepted or rejected, and why).
#[tokio::main]
async fn main() {
    env_logger::init();
    let mut args = std::env::args().skip(1);
    let providers = args.next().unwrap_or_else(|| "all".into());
    let img_path = args
        .next()
        .unwrap_or_else(|| "imgs/232265329-6a560438-e887-4f7f-b6a1-a61b8648f781.png".into());
    let providers = Arc::new(match providers.as_str() {
        "all" => all_providers(),
        "cpu" => vec![],
        _ => panic!("unknown provider set {providers}"),
    });
    let tagger = PixaiTagger::new(providers);
    let img = RawImage::new(&img_path).expect("image");
    let ip = Arc::new(CpuImageProcessor::default()) as Arc<dyn ImageOp + Send + Sync>;

    let load = Instant::now();
    let mut times = Vec::new();
    let mut tags = String::new();
    for i in 0..WARMUP + ITERS {
        let t0 = Instant::now();
        let out = tagger
            .tag(&img, TaggerOptions::default(), &ip)
            .await
            .expect("tag");
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        if i == 0 {
            eprintln!(
                "first call incl. session load {:.0}ms",
                load.elapsed().as_secs_f64() * 1000.0
            );
        }
        tags = out.to_string();
        if i >= WARMUP {
            times.push(ms);
        }
        eprintln!(
            "iter={} {:.1}ms general={} character={}{}",
            i,
            ms,
            out.general.len(),
            out.character.len(),
            if i < WARMUP { " warmup" } else { "" }
        );
    }
    let min = times.iter().copied().fold(f64::INFINITY, f64::min);
    let mean = times.iter().sum::<f64>() / times.len() as f64;
    println!(
        "bench_tag img={} {}x{} iters={} min_ms={:.1} median_ms={:.1} mean_ms={:.1}\n{}",
        img_path,
        img.width,
        img.height,
        ITERS,
        min,
        median(times),
        mean,
        tags
    );
}
