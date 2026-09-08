use std::sync::Arc;
use std::time::Instant;

use base_util::onnx::all_providers;
use interface_detector::{DefaultOptions, Detector, PreprocessorOptions};
use interface_image::{CpuImageProcessor, ImageOp, RawImage};
use interface_ocr::{Ocr, OcrOptions};
use parking_lot::Mutex;

const WARMUP: usize = 2;
const ITERS: usize = 5;
const MAX_BATCH_SIZE: usize = 16;

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

/// Usage: bench_ocr [ocr48px|ctc48px|mangaocr|native] [cpu|coreml] [image path relative to repo root]
///
/// Text lines come from a CPU-only ctd pass over the page, so the OCR input is the same set of
/// crops the pipeline feeds it, independent of the OCR provider under test.
#[tokio::main]
async fn main() {
    env_logger::init();
    let mut args = std::env::args().skip(1);
    let which = args.next().unwrap_or_else(|| "ocr48px".into());
    let provider = args.next().unwrap_or_else(|| "coreml".into());
    let path = args
        .next()
        .unwrap_or_else(|| "imgs/232265329-6a560438-e887-4f7f-b6a1-a61b8648f781.png".into());
    let providers = Arc::new(match provider.as_str() {
        "cpu" => Vec::new(),
        "coreml" => all_providers(),
        _ => panic!("unknown provider {provider}"),
    });
    let ocr: Box<dyn Ocr + Send + Sync> = match which.as_str() {
        "ocr48px" => Box::new(ocr_48px::Ocr48px::new(providers, 256, MAX_BATCH_SIZE)),
        "ctc48px" => Box::new(ctc_48px::Ctc48pxOcr::new(providers, MAX_BATCH_SIZE)),
        "mangaocr" => Box::new(manga_ocr::MangaOCR::new(providers, 256)),
        "native" => Box::new(native::NativeOCR::default()),
        _ => panic!("unknown ocr {which}"),
    };
    let img = RawImage::new(&path).expect("image");
    let ip = Arc::new(CpuImageProcessor::default()) as Arc<dyn ImageOp + Send + Sync>;
    let (boxes, _) = ctd::CtdDetector::new(Arc::new(Vec::new()))
        .detect(
            &img,
            PreprocessorOptions::default(),
            DefaultOptions::default(),
            &ip,
        )
        .await
        .expect("detect");
    let areas: Vec<_> = boxes.into_iter().map(|q| Arc::new(Mutex::new(q))).collect();

    let mut times = Vec::new();
    let mut fingerprint = String::new();
    for i in 0..WARMUP + ITERS {
        let t0 = Instant::now();
        let lines = ocr
            .detect(&img, &areas, OcrOptions::default(), &ip)
            .await
            .expect("ocr");
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        let mut rows: Vec<(i64, i64, String, f64)> = lines
            .iter()
            .map(|l| {
                let (x, y, _, _) = l.pos.lock().xyxy();
                (x, y, l.text.clone(), l.prob)
            })
            .collect();
        rows.sort_by(|a, b| (a.0, a.1, &a.2).cmp(&(b.0, b.1, &b.2)));
        let text_fnv = fnv(rows.iter().flat_map(|r| r.2.bytes().chain([0u8])));
        fingerprint = format!("lines={} text_fnv={:016x}", rows.len(), text_fnv);
        if i >= WARMUP {
            times.push(ms);
        }
        if let Ok(dump) = std::env::var("BENCH_DUMP") {
            let body: Vec<String> = rows
                .iter()
                .map(|(x, y, t, p)| format!("{x}\t{y}\t{p:.6}\t{t}"))
                .collect();
            std::fs::write(&dump, body.join("\n")).expect("dump lines");
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
        "bench_ocr ocr={} provider={} img={} {}x{} areas={} iters={} min_ms={:.1} median_ms={:.1} mean_ms={:.1} {}",
        which,
        provider,
        path,
        img.width,
        img.height,
        areas.len(),
        ITERS,
        min,
        median(times),
        mean,
        fingerprint
    );
}
