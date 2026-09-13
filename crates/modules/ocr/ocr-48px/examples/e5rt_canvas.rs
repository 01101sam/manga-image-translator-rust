//! P0 padding 对照 + 格式/EP 对照。不改 prepare / infer / reload。
//! `e5rt_canvas <cpu|coreml|coreml-nn|coreml-mlp> <raw|pad|both> <page_a.png> <page_b.png>`
//! `coreml` 与 `coreml-mlp` 都走 `new_session_mlprogram`。`coreml-nn` 走 `new_session`（NeuralNetwork）。
//! example 用同源 infer.rs。假行会进 infer 的 n，只把 real_n 配给区域。

#[path = "../src/hypo.rs"]
mod hypo;
#[path = "../src/infer.rs"]
mod infer;

use std::ffi::CStr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use base_util::onnx::{all_providers, new_session, new_session_mlprogram, Providers};
use base_util::project::root_path;
use infer::Pred;
use interface_detector::textlines::Quadrilateral;
use interface_image::RawImage;
use ndarray::{s, Array2, Array4};
use ort::inputs;
use ort::logging::LogLevel;
use ort::session::builder::SessionBuilder;
use ort::session::Session;
use ort::value::Tensor;
use parking_lot::Mutex as ParkMutex;

#[derive(Clone, Copy)]
enum EncKind {
    Cpu,
    NeuralNetwork,
    MlProgram,
}

const MAX_SEQ_LEN: i32 = 255;
const MAX_BATCH: usize = 2;
const CANVAS_N: usize = 2;
const CANVAS_W: usize = 283;
const PAD: f32 = -1.0;
const PAGE_A_TEXTS: [&str; 3] = ["そうだなあ‥", "ふふっ、", "ふふっ、"];

fn emit(kind: &str, fields: impl AsRef<str>) {
    eprintln!("E5RT_CANVAS {kind} {}", fields.as_ref());
}

fn rss_kb() -> u64 {
    let pid = std::process::id();
    std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

fn native_ort_version() -> String {
    unsafe {
        let base = ort::sys::OrtGetApiBase();
        if base.is_null() {
            return "null".into();
        }
        let ptr = ((*base).GetVersionString)();
        if ptr.is_null() {
            return "null".into();
        }
        CStr::from_ptr(ptr).to_string_lossy().into_owned()
    }
}

fn feat_w(w: i32) -> i32 {
    (w + 3) / 4 + 2
}

fn host_mask(widths: &[i32], seq: usize) -> Array2<bool> {
    Array2::from_shape_fn((widths.len(), seq), |(b, t)| {
        (t as i32) >= feat_w(widths[b])
    })
}

fn pred_text(pred: &Pred, dict: &[String]) -> String {
    let mut out = String::new();
    for &chid in &pred.out_idx {
        let ch = dict.get(chid as usize).map(String::as_str).unwrap_or("");
        if ch == "<S>" {
            continue;
        } else if ch == "</S>" {
            break;
        } else if ch == "<SP>" {
            out.push(' ');
        } else {
            out.push_str(ch);
        }
    }
    out
}

fn box_pts(xyxy: (i64, i64, i64, i64)) -> Vec<(i64, i64)> {
    let (x1, y1, x2, y2) = xyxy;
    vec![(x1, y1), (x2, y1), (x2, y2), (x1, y2)]
}

fn areas(boxes: &[(i64, i64, i64, i64)]) -> Vec<Arc<ParkMutex<Quadrilateral>>> {
    boxes
        .iter()
        .copied()
        .map(|xy| Arc::new(ParkMutex::new(Quadrilateral::new(box_pts(xy), 1.0))))
        .collect()
}

fn page_a_boxes() -> Vec<(i64, i64, i64, i64)> {
    vec![
        (208, 4, 246, 192),
        (76, 1788, 128, 1930),
        (76, 1788, 128, 1930),
    ]
}

fn page_b_boxes() -> Vec<(i64, i64, i64, i64)> {
    vec![
        (2100, 60, 2260, 980),
        (50, 70, 190, 430),
        (50, 2280, 230, 3040),
    ]
}

fn pad_to_canvas(
    img: &Array4<f32>,
    widths: &[i32],
) -> Result<(Array4<f32>, Vec<i32>, usize), String> {
    let n = img.shape()[0];
    let c = img.shape()[1];
    let h = img.shape()[2];
    let w = img.shape()[3];
    if n > CANVAS_N || w > CANVAS_W {
        return Err(format!("overflow n={n} w={w} canvas={CANVAS_N}x{CANVAS_W}"));
    }
    let mut out = Array4::from_elem((CANVAS_N, c, h, CANVAS_W), PAD);
    out.slice_mut(s![..n, .., .., ..w]).assign(img);
    let mut new_w = vec![0i32; CANVAS_N];
    new_w[..n].copy_from_slice(widths);
    Ok((out, new_w, n))
}

struct EncOut {
    mem_shape: Vec<usize>,
    mask_shape: Vec<usize>,
    mask: Array2<bool>,
    mem: ndarray::Array3<f32>,
}

fn run_encoder(enc: &Mutex<Session>, img: Array4<f32>, widths: Vec<i32>) -> EncOut {
    let mut enc = enc.lock().expect("encoder mutex");
    let out = enc
        .run(inputs! {
            "img" => Tensor::from_array(img).unwrap(),
            "img_widths" => Tensor::from_array(ndarray::Array1::from(widths)).unwrap(),
        })
        .unwrap();
    let mem = out
        .get("memory")
        .unwrap()
        .try_extract_array::<f32>()
        .unwrap()
        .to_owned();
    let mask = out
        .get("input_mask")
        .unwrap()
        .try_extract_array::<bool>()
        .unwrap()
        .to_owned();
    EncOut {
        mem_shape: mem.shape().to_vec(),
        mask_shape: mask.shape().to_vec(),
        mask: mask.into_dimensionality::<ndarray::Ix2>().unwrap(),
        mem: mem.into_dimensionality::<ndarray::Ix3>().unwrap(),
    }
}

fn require_layout(n: usize, w: usize, enc: &EncOut) -> Result<usize, String> {
    if enc.mem_shape.len() != 3 || enc.mem_shape[0] != n || enc.mem_shape[2] != 320 {
        return Err(format!("memory want=[{n},seq,320] got={:?}", enc.mem_shape));
    }
    let seq = enc.mem_shape[1];
    if w % 2 == 0 && seq != w / 4 {
        return Err(format!("seq want W/4={} got={seq} W={w}", w / 4));
    }
    if enc.mask_shape != [n, seq] {
        return Err(format!("mask want=[{n},{seq}] got={:?}", enc.mask_shape));
    }
    Ok(seq)
}

fn mask_report(tag: &str, widths: &[i32], enc: &EncOut) -> Result<(), String> {
    let n = widths.len();
    if enc.mask_shape.len() != 2 || enc.mask_shape[0] != n {
        return Err(format!(
            "{tag} mask want=[{n},seq] got={:?}",
            enc.mask_shape
        ));
    }
    let seq = enc.mask_shape[1];
    let host = host_mask(widths, seq);
    let mut mism = 0usize;
    for b in 0..n {
        for t in 0..seq {
            if host[(b, t)] != enc.mask[(b, t)] {
                mism += 1;
            }
        }
    }
    let feats: Vec<i32> = widths.iter().copied().map(feat_w).collect();
    emit(
        "mask",
        format!(
            "{tag} widths={widths:?} feat={feats:?} enc_mask={:?} n_ax=0 seq_ax=1 host_seq={seq} mismatches={mism}",
            enc.mask_shape
        ),
    );
    if mism != 0 {
        return Err(format!("{tag} mask mismatches={mism}"));
    }
    Ok(())
}

fn place_texts(
    dest: &mut [Option<String>],
    texts: &[String],
    batch_areas: &[Arc<ParkMutex<Quadrilateral>>],
    page_areas: &[Arc<ParkMutex<Quadrilateral>>],
) {
    for (text, area) in texts.iter().zip(batch_areas.iter()) {
        if let Some(i) = page_areas.iter().enumerate().find_map(|(i, orig)| {
            if dest[i].is_none() && Arc::ptr_eq(orig, area) {
                Some(i)
            } else {
                None
            }
        }) {
            dest[i] = Some(text.clone());
        }
    }
}

fn infer_real(
    enc: &Mutex<Session>,
    dec: &Mutex<Session>,
    color: &Mutex<Session>,
    img: Array4<f32>,
    widths: Vec<i32>,
    real_n: usize,
    dict: &[String],
) -> Vec<String> {
    let n_in = img.shape()[0];
    let preds = infer::infer(enc, dec, color, img, widths, 1, 2, 5, MAX_SEQ_LEN, 2);
    emit(
        "decode",
        format!(
            "img_n={n_in} pred_n={} real_n={real_n} dummy_decoded={}",
            preds.len(),
            preds.len().saturating_sub(real_n)
        ),
    );
    preds
        .iter()
        .take(real_n)
        .map(|p| pred_text(p, dict))
        .collect()
}

fn mem_valid_diff(
    raw: &EncOut,
    pad: &EncOut,
    raw_n: usize,
    raw_w: usize,
    pad_n: usize,
    pad_w: usize,
) -> Result<(f32, usize), String> {
    let seq_raw = require_layout(raw_n, raw_w, raw)?;
    let seq_pad = require_layout(pad_n, pad_w, pad)?;
    if seq_pad < seq_raw {
        return Err(format!("pad seq {seq_pad} < raw seq {seq_raw}"));
    }
    if raw.mem.shape()[2] != 320 || pad.mem.shape()[2] != 320 {
        return Err("memory last dim is not 320".into());
    }
    let expected = raw_n * seq_raw * 320;
    let mut max_abs = 0.0f32;
    let mut compared = 0usize;
    for b in 0..raw_n {
        for t in 0..seq_raw {
            for c in 0..320 {
                let d = (raw.mem[(b, t, c)] - pad.mem[(b, t, c)]).abs();
                max_abs = max_abs.max(d);
                compared += 1;
            }
        }
    }
    if compared != expected {
        return Err(format!("compared={compared} want={expected}"));
    }
    Ok((max_abs, compared))
}

fn attach_ort_logger(builder: SessionBuilder) -> SessionBuilder {
    builder
        .with_logger(Box::new(
            |level: LogLevel, category: &str, id: &str, loc: &str, message: &str| {
                emit(
                    "ort",
                    format!("level={level:?} category={category} id={id} loc={loc} msg={message}"),
                );
            },
        ))
        .and_then(|b| b.with_log_level(LogLevel::Verbose))
        .expect("ort logger")
}

fn parse_kind(name: &str) -> Result<EncKind, String> {
    match name {
        "cpu" => Ok(EncKind::Cpu),
        "coreml" | "coreml-mlp" | "mlprogram" => Ok(EncKind::MlProgram),
        "coreml-nn" | "nn" => Ok(EncKind::NeuralNetwork),
        other => Err(format!("unknown provider {other}")),
    }
}

fn load_sessions(kind: EncKind) -> (Mutex<Session>, Mutex<Session>, Mutex<Session>, Vec<String>) {
    let root = root_path();
    let enc_p = root.join("models/ocr/48px/encoder.onnx");
    let dec_p = root.join("models/ocr/48px/decoder.onnx");
    let col_p = root.join("models/ocr/48px/color_pred.onnx");
    let abc_p = root.join("models/ocr/48px/alphabet-all-v7.txt");
    let enc_prov = match kind {
        EncKind::Cpu => Vec::new(),
        EncKind::NeuralNetwork | EncKind::MlProgram => all_providers(),
    };
    let no_coreml: Vec<Providers> = enc_prov
        .iter()
        .filter(|p| !matches!(p, Providers::CoreML))
        .cloned()
        .collect();
    let (ctor, format) = match kind {
        EncKind::Cpu => ("new_session", "none_cpu"),
        EncKind::NeuralNetwork => ("new_session", "NeuralNetwork"),
        EncKind::MlProgram => ("new_session_mlprogram", "MLProgram"),
    };
    emit(
        "session",
        format!(
            "ctor={ctor} requested_format={format} encoder_providers={:?} decoder_providers={:?} cache_dir=models/cache compute_units=CPUAndGPU context7=no_namespace ort_src=ort-2.0.0-rc.10",
            enc_prov, no_coreml
        ),
    );
    let t0 = Instant::now();
    let enc_builder = match kind {
        EncKind::MlProgram => new_session_mlprogram(&enc_prov).unwrap(),
        EncKind::Cpu | EncKind::NeuralNetwork => new_session(&enc_prov).unwrap(),
    };
    let enc = attach_ort_logger(enc_builder)
        .commit_from_file(&enc_p)
        .unwrap();
    let dec = attach_ort_logger(new_session(&no_coreml).unwrap())
        .commit_from_file(&dec_p)
        .unwrap();
    let col = attach_ort_logger(new_session(&no_coreml).unwrap())
        .commit_from_file(&col_p)
        .unwrap();
    emit(
        "load",
        format!(
            "ms={:.3} rss_kb={} ctor={ctor} requested_format={format} enc_exists={} native={}",
            t0.elapsed().as_secs_f64() * 1000.0,
            rss_kb(),
            enc_p.exists(),
            native_ort_version()
        ),
    );
    let dict = std::fs::read_to_string(abc_p)
        .unwrap()
        .lines()
        .map(|v| v.trim_end().to_string())
        .collect();
    (Mutex::new(enc), Mutex::new(dec), Mutex::new(col), dict)
}

fn run_path(
    enc: &Mutex<Session>,
    dec: &Mutex<Session>,
    col: &Mutex<Session>,
    dict: &[String],
    img: Array4<f32>,
    widths: Vec<i32>,
    batch_n: usize,
    pad: bool,
    tag: &str,
) -> Result<(Vec<String>, f64, f64), String> {
    let real_n = img.shape()[0];
    let (use_img, use_w, real_n) = if pad {
        pad_to_canvas(&img, &widths)?
    } else {
        (img, widths, real_n)
    };
    if real_n != batch_n {
        return Err(format!("{tag} real_n={real_n} batch_n={batch_n}"));
    }
    emit(
        "tensor",
        format!(
            "{tag} pad={pad} img={:?} widths={:?} real_n={real_n} fill={}",
            use_img.shape(),
            use_w,
            if pad { PAD } else { f32::NAN }
        ),
    );
    if pad && real_n == 1 {
        emit(
            "tail",
            format!(
                "{tag} n1_padded_n2 real_n=1 img_n={} dummy_width={}",
                use_img.shape()[0],
                use_w.get(1).copied().unwrap_or(-1)
            ),
        );
    }
    let t = Instant::now();
    let texts = infer_real(enc, dec, col, use_img.clone(), use_w.clone(), real_n, dict);
    let infer_ms = t.elapsed().as_secs_f64() * 1000.0;
    let skip_diag = std::env::var("E5RT_CANVAS_SKIP_ENC_DIAG").ok().as_deref() == Some("1");
    let enc_diag_ms = if skip_diag {
        0.0
    } else {
        let t = Instant::now();
        let enc_out = run_encoder(enc, use_img.clone(), use_w.clone());
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        let w = use_img.shape()[3];
        let n = use_img.shape()[0];
        require_layout(n, w, &enc_out)?;
        emit(
            "enc",
            format!(
                "{tag} pad={pad} img={:?} memory={:?} input_mask={:?} W={w} w_div4={} feat0={} n_ax=0 seq_ax=1 seq_obs={} enc_diag_ms={ms:.3} after_infer=1",
                use_img.shape(),
                enc_out.mem_shape,
                enc_out.mask_shape,
                w / 4,
                feat_w(use_w[0]),
                enc_out.mask.shape()[1]
            ),
        );
        mask_report(tag, &use_w, &enc_out)?;
        ms
    };
    emit(
        "timing",
        format!("{tag} pad={pad} infer_ms={infer_ms:.3} enc_diag_ms={enc_diag_ms:.3} infer_before_enc_diag=1"),
    );
    Ok((texts, infer_ms, enc_diag_ms))
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let provider = args.next().unwrap_or_else(|| "cpu".into());
    let path = args.next().unwrap_or_else(|| "both".into());
    let page_a = args
        .next()
        .unwrap_or_else(|| "imgs/232265329-6a560438-e887-4f7f-b6a1-a61b8648f781.png".into());
    let page_b = args
        .next()
        .unwrap_or_else(|| "imgs/232264684-5a7bcf8e-707b-4925-86b0-4212382f1680.png".into());
    let kind = match parse_kind(&provider) {
        Ok(k) => k,
        Err(e) => {
            emit("fail", e);
            return ExitCode::from(2);
        }
    };
    let do_raw = path == "raw" || path == "both";
    let do_pad = path == "pad" || path == "both";
    let coreml = !matches!(kind, EncKind::Cpu);
    if path == "both" && coreml {
        emit("fail", "coreml_both_forbidden_separate_cache");
        return ExitCode::from(2);
    }

    emit(
        "meta",
        format!(
            "cwd={} root={} native={} build={} provider={provider} path={path} canvas={CANVAS_N}x{CANVAS_W} pad_fill={PAD} providers_source={} context7=no_namespace",
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")).display(),
            root_path().display(),
            native_ort_version(),
            ort::info(),
            if coreml { "all_providers_not_ep_registration" } else { "empty_cpu" }
        ),
    );

    let (enc, dec, col, dict) = load_sessions(kind);
    let mut errors = Vec::new();
    let mut first_a_raw: Option<Vec<String>> = None;
    let mut first_a_pad: Option<Vec<String>> = None;

    let pages = [
        ("A", page_a.as_str(), page_a_boxes(), true),
        ("B", page_b.as_str(), page_b_boxes(), false),
        ("A", page_a.as_str(), page_a_boxes(), true),
    ];

    for (visit_i, (page, img_path, boxes, oracle)) in pages.iter().enumerate() {
        let visit = if *page == "A" && visit_i == 2 { 2 } else { 1 };
        let raw_img = RawImage::new(img_path).expect("image");
        let area_arcs = areas(boxes);
        let prepared =
            util::ocr::prepare(&raw_img, &area_arcs, 48, MAX_BATCH, &None).expect("prepare");
        let t0 = Instant::now();
        let mut infer_ms = 0.0f64;
        let mut enc_diag_ms = 0.0f64;
        let mut raw_by_area: Vec<Option<String>> = vec![None; area_arcs.len()];
        let mut pad_by_area: Vec<Option<String>> = vec![None; area_arcs.len()];
        for (bi, (images, widths, batch_areas)) in prepared.into_iter().enumerate() {
            let tag = format!("page={page} visit={visit} batch={bi}");
            emit(
                "shape",
                format!("{tag} prepare_img={:?} widths={:?}", images.shape(), widths),
            );
            if do_raw {
                match run_path(
                    &enc,
                    &dec,
                    &col,
                    &dict,
                    images.clone(),
                    widths.clone(),
                    batch_areas.len(),
                    false,
                    &tag,
                ) {
                    Ok((texts, i_ms, d_ms)) => {
                        infer_ms += i_ms;
                        enc_diag_ms += d_ms;
                        place_texts(&mut raw_by_area, &texts, &batch_areas, &area_arcs);
                    }
                    Err(e) => errors.push(e),
                }
            }
            if do_pad {
                match run_path(
                    &enc,
                    &dec,
                    &col,
                    &dict,
                    images,
                    widths,
                    batch_areas.len(),
                    true,
                    &tag,
                ) {
                    Ok((texts, i_ms, d_ms)) => {
                        infer_ms += i_ms;
                        enc_diag_ms += d_ms;
                        place_texts(&mut pad_by_area, &texts, &batch_areas, &area_arcs);
                    }
                    Err(e) => errors.push(e),
                }
            }
        }
        let page_texts_raw: Vec<String> = raw_by_area
            .into_iter()
            .map(|t| t.unwrap_or_default())
            .collect();
        let page_texts_pad: Vec<String> = pad_by_area
            .into_iter()
            .map(|t| t.unwrap_or_default())
            .collect();
        emit(
            "page",
            format!(
                "page={page} visit={visit} ms={infer_ms:.3} infer_ms={infer_ms:.3} enc_diag_ms={enc_diag_ms:.3} wall_ms={:.3} rss_kb={} raw={:?} pad={:?} areas={} timing=infer_only_not_p0_double_encode",
                t0.elapsed().as_secs_f64() * 1000.0,
                rss_kb(),
                page_texts_raw,
                page_texts_pad,
                area_arcs.len()
            ),
        );
        let check = if do_pad {
            &page_texts_pad
        } else {
            &page_texts_raw
        };
        if *oracle {
            if check.len() != PAGE_A_TEXTS.len() {
                errors.push(format!(
                    "page=A visit={visit} count got={} want=3",
                    check.len()
                ));
            }
            for (i, (got, want)) in check.iter().zip(PAGE_A_TEXTS.iter()).enumerate() {
                if got.is_empty() {
                    errors.push(format!("page=A visit={visit} idx={i} missing_region"));
                } else if got != *want {
                    errors.push(format!(
                        "page=A visit={visit} idx={i} text got={got:?} want={want:?}"
                    ));
                }
                let (x1, y1, _, _) = area_arcs[i].lock().xyxy();
                emit(
                    "region",
                    format!("page=A visit={visit} idx={i} role=oracle xy={x1},{y1} text={got:?}"),
                );
            }
        } else {
            for (i, got) in check.iter().enumerate() {
                emit(
                    "region",
                    format!("page=B visit={visit} idx={i} role=diff text={got:?}"),
                );
            }
        }
        if *page == "A" && visit == 1 {
            if do_raw {
                first_a_raw = Some(page_texts_raw.clone());
            }
            if do_pad {
                first_a_pad = Some(page_texts_pad.clone());
            }
        }
        if *page == "A" && visit == 2 {
            if let (Some(a), true) = (&first_a_raw, do_raw) {
                if a != &page_texts_raw {
                    errors.push(format!("raw repeat mismatch {a:?} vs {page_texts_raw:?}"));
                }
            }
            if let (Some(a), true) = (&first_a_pad, do_pad) {
                if a != &page_texts_pad {
                    errors.push(format!("pad repeat mismatch {a:?} vs {page_texts_pad:?}"));
                }
            }
        }
        if do_raw && do_pad && *oracle && page_texts_raw != page_texts_pad {
            errors.push(format!(
                "page=A visit={visit} pad_broke_text raw={page_texts_raw:?} pad={page_texts_pad:?}"
            ));
        }
    }

    if do_raw && do_pad {
        let img = RawImage::new(&page_a).unwrap();
        let a = areas(&page_a_boxes());
        let prepared = util::ocr::prepare(&img, &a, 48, MAX_BATCH, &None).unwrap();
        if let Some((images, widths, _)) = prepared.into_iter().next() {
            let raw_n = images.shape()[0];
            let raw_w = images.shape()[3];
            let raw_e = run_encoder(&enc, images.clone(), widths.clone());
            let (pimg, pw, real_n) = pad_to_canvas(&images, &widths).unwrap();
            let pad_n = pimg.shape()[0];
            let pad_w = pimg.shape()[3];
            let pad_e = run_encoder(&enc, pimg, pw);
            match (
                require_layout(raw_n, raw_w, &raw_e),
                require_layout(pad_n, pad_w, &pad_e),
            ) {
                (Ok(_), Ok(_)) => emit(
                    "axes",
                    format!(
                        "raw_mem={:?} pad_mem={:?} n_ax=0 seq_ax=1 raw_n={real_n} contract=[N,seq,320]",
                        raw_e.mem_shape, pad_e.mem_shape
                    ),
                ),
                (e1, e2) => errors.push(format!("layout raw={e1:?} pad={e2:?}")),
            }
            match mem_valid_diff(&raw_e, &pad_e, raw_n, raw_w, pad_n, pad_w) {
                Ok((max_abs, ncmp)) => emit(
                    "memdiff",
                    format!("valid_max_abs={max_abs} compared={ncmp} note=shorter_seq_full_Nc_not_full_tensor"),
                ),
                Err(e) => errors.push(format!("memdiff {e}")),
            }
            let zero_w = run_encoder(&enc, images.clone(), vec![0; widths.len()]);
            emit(
                "mask_in_memory",
                format!(
                    "same_img_zero_widths memory_changed={} mask_changed={}",
                    raw_e.mem != zero_w.mem,
                    raw_e.mask != zero_w.mask
                ),
            );
            let canvas = Array4::from_elem((CANVAS_N, 3, 48, CANVAS_W), PAD);
            for w in [0, 1, 2, 131, 237, 276, 283] {
                let ws = vec![w, 0];
                let e = run_encoder(&enc, canvas.clone(), ws.clone());
                if let Err(err) = require_layout(CANVAS_N, CANVAS_W, &e) {
                    errors.push(format!("probe_w={w} {err}"));
                }
                if let Err(err) = mask_report(&format!("probe_w={w}"), &ws, &e) {
                    errors.push(err);
                }
                emit(
                    "seq",
                    format!(
                        "probe_w={w} memory={:?} mask={:?} feat0={} w_over_4={}",
                        e.mem_shape,
                        e.mask_shape,
                        feat_w(w),
                        w / 4
                    ),
                );
            }
        }
    }

    if errors.is_empty() {
        emit("accept", "pass");
        ExitCode::SUCCESS
    } else {
        for e in &errors {
            emit("fail", e);
        }
        ExitCode::from(1)
    }
}
