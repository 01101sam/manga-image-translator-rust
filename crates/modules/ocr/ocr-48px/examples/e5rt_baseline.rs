//! 未修改生产路径上的 OCR48px 动态 CoreML 编码器基线。
//! `e5rt_baseline <page_a.png> <page_b.png>`
//! 结构化记录写 stderr，前缀 `E5RT_BASELINE`。E5RT 仍走 stdout。

use std::ffi::CStr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;

use base_util::onnx::all_providers;
use base_util::project::root_path;
use interface_detector::textlines::Quadrilateral;
use interface_image::{CpuImageProcessor, ImageOp, RawImage};
use interface_model::ModelLoad;
use interface_ocr::Ocr as _;
use ocr_48px::Ocr48px;
use parking_lot::Mutex;

const MAX_SEQ_LEN: i32 = 255;
const MAX_BATCH: usize = 2;
/// 页 A 金标准，与 `ocr_full_then_partial_batch` 的输入 index 一一对应。
const PAGE_A_TEXTS: [&str; 3] = ["そうだなあ‥", "ふふっ、", "ふふっ、"];

fn emit(kind: &str, fields: impl AsRef<str>) {
    eprintln!("E5RT_BASELINE {kind} {}", fields.as_ref());
}

fn rss_kb() -> u64 {
    let pid = std::process::id();
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .ok();
    out.and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

fn native_ort_version() -> String {
    unsafe {
        let base = ort::sys::OrtGetApiBase();
        if base.is_null() {
            return "null-OrtGetApiBase".into();
        }
        let ptr = ((*base).GetVersionString)();
        if ptr.is_null() {
            return "null-GetVersionString".into();
        }
        CStr::from_ptr(ptr).to_string_lossy().into_owned()
    }
}

fn file_meta(path: &Path) -> String {
    match std::fs::metadata(path) {
        Ok(meta) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                format!(
                    "path={} exists=1 size={} inode={} nlink={} mode={:o}",
                    path.display(),
                    meta.len(),
                    meta.ino(),
                    meta.nlink(),
                    meta.mode()
                )
            }
            #[cfg(not(unix))]
            {
                format!("path={} exists=1 size={}", path.display(), meta.len())
            }
        }
        Err(_) => format!("path={} exists=0", path.display()),
    }
}

fn box_pts(xyxy: (i64, i64, i64, i64)) -> Vec<(i64, i64)> {
    let (x1, y1, x2, y2) = xyxy;
    vec![(x1, y1), (x2, y1), (x2, y2), (x1, y2)]
}

fn areas(boxes: &[(i64, i64, i64, i64)]) -> Vec<Arc<Mutex<Quadrilateral>>> {
    boxes
        .iter()
        .copied()
        .map(|xyxy| Arc::new(Mutex::new(Quadrilateral::new(box_pts(xyxy), 1.0))))
        .collect()
}

fn page_a_boxes() -> Vec<(i64, i64, i64, i64)> {
    vec![
        (208, 4, 246, 192),
        (76, 1788, 128, 1930),
        (76, 1788, 128, 1930),
    ]
}

/// 页 B 固定原图框，只做差分输出，不验收文字。
fn page_b_boxes() -> Vec<(i64, i64, i64, i64)> {
    vec![
        (2100, 60, 2260, 980),
        (50, 70, 190, 430),
        (50, 2280, 230, 3040),
    ]
}

fn describe_batches(image: &RawImage, boxes: &[Arc<Mutex<Quadrilateral>>], page: &str) {
    let prepared = util::ocr::prepare(image, boxes, 48, MAX_BATCH, &None).expect("prepare");
    for (batch_i, (images, widths, batch_areas)) in prepared.iter().enumerate() {
        let shape = images.shape();
        let verts: Vec<u8> = batch_areas
            .iter()
            .map(|a| u8::from(a.lock().vertical()))
            .collect();
        emit(
            "shape",
            format!(
                "page={page} batch={batch_i} n={} c={} h={} w={} img_widths={:?} vertical={:?}",
                shape[0], shape[1], shape[2], shape[3], widths, verts
            ),
        );
    }
}

struct PageResult {
    texts: Vec<String>,
    errors: Vec<String>,
}

async fn run_page(
    ocr: &Ocr48px,
    ip: &Arc<dyn ImageOp + Send + Sync>,
    page: &str,
    path: &str,
    boxes: &[(i64, i64, i64, i64)],
    visit: usize,
    expected_texts: Option<&[&str]>,
) -> PageResult {
    let img = RawImage::new(path).expect("image");
    let areas = areas(boxes);
    describe_batches(&img, &areas, page);
    let t0 = Instant::now();
    let lines = ocr
        .detect(&img, &areas, Default::default(), ip)
        .await
        .expect("ocr detect");
    let ms = t0.elapsed().as_secs_f64() * 1000.0;
    emit(
        "page",
        format!(
            "page={page} visit={visit} img={} ms={ms:.3} rss_kb={} regions={}",
            Path::new(path)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy(),
            rss_kb(),
            lines.len()
        ),
    );

    let mut errors = Vec::new();
    if lines.len() != areas.len() {
        errors.push(format!(
            "page={page} visit={visit} region_count got={} want={}",
            lines.len(),
            areas.len()
        ));
    }

    let mut texts = Vec::with_capacity(areas.len());
    for (i, (area, expected_box)) in areas.iter().zip(boxes.iter()).enumerate() {
        let hit = lines.iter().find(|line| Arc::ptr_eq(&line.pos, area));
        let Some(hit) = hit else {
            errors.push(format!("page={page} visit={visit} idx={i} missing_region"));
            texts.push(String::new());
            continue;
        };
        let (x1, y1, _x2, _y2) = hit.pos.lock().xyxy();
        let role = if expected_texts.is_some() {
            "oracle"
        } else {
            "diff"
        };
        emit(
            "region",
            format!(
                "page={page} visit={visit} idx={i} role={role} box={},{},{},{} xyxy_x={x1} xyxy_y={y1} text={:?} prob={:.6}",
                expected_box.0,
                expected_box.1,
                expected_box.2,
                expected_box.3,
                hit.text,
                hit.prob
            ),
        );
        if let Some(expected) = expected_texts.and_then(|xs| xs.get(i)).copied() {
            if hit.text != expected {
                errors.push(format!(
                    "page={page} visit={visit} idx={i} text got={:?} want={expected:?}",
                    hit.text
                ));
            }
        }
        texts.push(hit.text.clone());
    }
    PageResult { texts, errors }
}

#[tokio::main]
async fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let page_a = args
        .next()
        .unwrap_or_else(|| "imgs/232265329-6a560438-e887-4f7f-b6a1-a61b8648f781.png".into());
    let page_b = args
        .next()
        .unwrap_or_else(|| "imgs/232264684-5a7bcf8e-707b-4925-86b0-4212382f1680.png".into());

    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "unknown".into());
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let encoder = root_path().join("models/ocr/48px/encoder.onnx");
    let decoder = root_path().join("models/ocr/48px/decoder.onnx");
    let color_pred = root_path().join("models/ocr/48px/color_pred.onnx");
    let alphabet = root_path().join("models/ocr/48px/alphabet-all-v7.txt");

    emit("meta", format!("exe={exe}"));
    emit("meta", format!("cwd={}", cwd.display()));
    emit("meta", format!("root_path={}", root_path().display()));
    emit(
        "meta",
        format!(
            "ort_crate_minor={} native_GetVersionString={} build_info={}",
            ort::MINOR_VERSION,
            native_ort_version(),
            ort::info()
        ),
    );
    emit(
        "meta",
        format!(
            "providers_requested={:?} providers_source=all_providers_not_ep_registration coreml_options_source=base_util_new_session_ source=not_registered_ep_proof",
            all_providers()
        ),
    );
    emit("meta", format!("encoder {}", file_meta(&encoder)));
    emit("meta", format!("decoder {}", file_meta(&decoder)));
    emit("meta", format!("color_pred {}", file_meta(&color_pred)));
    emit("meta", format!("alphabet {}", file_meta(&alphabet)));
    emit(
        "meta",
        format!(
            "cargo_manifest_dir={}",
            std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| "".into())
        ),
    );

    // `E5RT_PAGE_A_EXPECT=a|b|c` 只用于故意失败探测，不是生产夹具。
    let expected_owned: Option<Vec<String>> = std::env::var("E5RT_PAGE_A_EXPECT")
        .ok()
        .filter(|s| !s.is_empty())
        .map(|s| s.split('|').map(|p| p.to_string()).collect());
    if let Some(parts) = expected_owned.as_ref() {
        if parts.len() != 3 {
            emit(
                "fail",
                format!("page_a_expect_override_len={} want=3", parts.len()),
            );
            return ExitCode::from(2);
        }
        emit("meta", "page_a_expect_override=1");
    }
    let expected_refs: [&str; 3] = match expected_owned.as_ref() {
        Some(parts) => [&parts[0], &parts[1], &parts[2]],
        None => PAGE_A_TEXTS,
    };

    let ocr = Ocr48px::new(Arc::new(all_providers()), MAX_SEQ_LEN, MAX_BATCH);
    let ip = Arc::new(CpuImageProcessor::default()) as Arc<dyn ImageOp + Send + Sync>;

    let t0 = Instant::now();
    ocr.load().await.expect("load sessions");
    emit(
        "load",
        format!(
            "ms={:.3} rss_kb={}",
            t0.elapsed().as_secs_f64() * 1000.0,
            rss_kb()
        ),
    );

    let a = page_a_boxes();
    let b = page_b_boxes();
    let first_a = run_page(&ocr, &ip, "A", &page_a, &a, 1, Some(&expected_refs)).await;
    let page_b = run_page(&ocr, &ip, "B", &page_b, &b, 1, None).await;
    let second_a = run_page(&ocr, &ip, "A", &page_a, &a, 2, Some(&expected_refs)).await;

    let mut errors = Vec::new();
    errors.extend(first_a.errors);
    errors.extend(page_b.errors);
    errors.extend(second_a.errors);
    if first_a.texts != second_a.texts {
        errors.push(format!(
            "page=A repeat_mismatch first={:?} second={:?}",
            first_a.texts, second_a.texts
        ));
    }
    if errors.is_empty() {
        emit("accept", "page_a_oracle=pass page_b_diff=pass repeat=pass");
        ExitCode::SUCCESS
    } else {
        for err in &errors {
            emit("fail", err);
        }
        ExitCode::from(1)
    }
}
