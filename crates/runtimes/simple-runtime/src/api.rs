use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Instant,
};

use actix_files::NamedFile;
use actix_multipart::form::{
    bytes::Bytes as MpBytes, tempfile::TempFile, MultipartForm, MultipartFormConfig,
};
use actix_web::{
    get, http::StatusCode, post,
    web::{self},
    App, HttpRequest, HttpResponse, HttpServer, Responder,
};
use html::HtmlRenderer;
use png::PngRenderer;
use uuid::Uuid;

use crate::{
    render_config,
    settings::{Renderer, Settings},
    setup::Models,
};

struct TranslateJob {
    img: image::DynamicImage,
    settings: Arc<Settings>,
    mask_out: Option<PathBuf>,
    reply: tokio::sync::oneshot::Sender<anyhow::Result<Option<export::Export>>>,
}

struct AppState {
    jobs: tokio::sync::mpsc::Sender<TranslateJob>,
    started: Instant,
    inflight: AtomicUsize,
}

struct Inflight<'a>(&'a AtomicUsize);

impl Drop for Inflight<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

struct TmpDir(PathBuf);

impl Drop for TmpDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn fail(status: StatusCode, error: impl Into<String>) -> HttpResponse {
    HttpResponse::build(status).json(serde_json::json!({
        "ok": false,
        "error": error.into(),
    }))
}

fn b64_encode(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let a = chunk[0] as u32;
        let b = chunk.get(1).copied().unwrap_or(0) as u32;
        let c = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (a << 16) | (b << 8) | c;
        out.push(T[(n >> 18) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            T[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

fn field_text(field: Option<&MpBytes>) -> String {
    field
        .map(|b| String::from_utf8_lossy(&b.data).into_owned())
        .unwrap_or_default()
}

fn parse_settings(raw: &str) -> Result<Settings, String> {
    let value: serde_json::Value = if raw.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(raw).map_err(|_| "参数 JSON 无效".to_string())?
    };
    if let Some(r) = value.pointer("/render/renderer") {
        match r.as_str() {
            Some("Png" | "Html" | "Raw") => {}
            Some(s) => {
                return Err(format!("不支持的渲染器: {s}，仅支持 Png、Html、Raw"));
            }
            None => {
                return Err("不支持的渲染器: 渲染器必须是 Png、Html 或 Raw".into());
            }
        }
    }
    Ok(serde_json::from_value(value).unwrap_or_default())
}

fn mime_of(renderer: &Renderer) -> &'static str {
    match renderer {
        Renderer::Png => "image/png",
        Renderer::Html => "text/html",
        Renderer::Raw => "application/octet-stream",
    }
}

fn write_render(
    exp: export::Export,
    settings: &Settings,
    output: &Path,
) -> Result<(), String> {
    match settings.render.renderer {
        Renderer::Html => {
            let (data, _) = HtmlRenderer::render(vec![exp], None, false);
            File::create(output)
                .and_then(|mut f| f.write_all(&data))
                .map_err(|e| format!("写入渲染结果失败: {e}"))?;
        }
        Renderer::Raw => {
            File::create(output)
                .and_then(|mut f| f.write_all(&exp.export()))
                .map_err(|e| format!("写入渲染结果失败: {e}"))?;
        }
        Renderer::Png => {
            let mut png = PngRenderer::default();
            let img = png
                .render(exp, render_config(&settings.render))
                .map_err(|e| format!("渲染失败: {e}"))?;
            img.to_image()
                .map_err(|e| format!("编码 PNG 失败: {e}"))?
                .save(output)
                .map_err(|e| format!("保存 PNG 失败: {e}"))?;
        }
    }
    Ok(())
}

#[get("/")]
async fn hello() -> impl Responder {
    HttpResponse::Ok().body("Hello world!")
}

#[get("/health")]
async fn health(state: web::Data<AppState>) -> impl Responder {
    HttpResponse::Ok().json(serde_json::json!({
        "service": "simple-runtime-api",
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_s": state.started.elapsed().as_secs(),
        "busy": state.inflight.load(Ordering::Relaxed) > 0,
    }))
}

#[get("/defaults/detector")]
async fn defaults_detector() -> impl Responder {
    let settings = crate::settings::DetectorSettings::default();
    let str = serde_json::to_string(&settings).unwrap();
    HttpResponse::Ok().body(str)
}

#[get("/defaults/ocr")]
async fn defaults_ocr() -> impl Responder {
    let settings = crate::settings::OCRSettings::default();
    let str = serde_json::to_string(&settings).unwrap();
    HttpResponse::Ok().body(str)
}

#[get("/defaults/inpainter")]
async fn defaults_inpainter() -> impl Responder {
    let settings = crate::settings::InpainterSettings::default();
    let str = serde_json::to_string(&settings).unwrap();
    HttpResponse::Ok().body(str)
}

#[get("/defaults/colorizer")]
async fn defaults_colorizer() -> impl Responder {
    let settings = crate::settings::ColorizerSettings::default();
    let str = serde_json::to_string(&settings).unwrap();
    HttpResponse::Ok().body(str)
}

#[get("/defaults/render")]
async fn defaults_render() -> impl Responder {
    let settings = crate::settings::RenderSettings::default();
    let str = serde_json::to_string(&settings).unwrap();
    HttpResponse::Ok().body(str)
}

#[get("/defaults/mask_refinement")]
async fn defaults_mask_refinement() -> impl Responder {
    let settings = crate::settings::MaskRefinementSettings::default();
    let str = serde_json::to_string(&settings).unwrap();
    HttpResponse::Ok().body(str)
}

#[get("/defaults/translator")]
async fn defaults_translator() -> impl Responder {
    let settings = crate::settings::TranslatorSettings::default();
    let str = serde_json::to_string(&settings).unwrap();
    HttpResponse::Ok().body(str)
}

const UPLOAD_DIR: &str = "./uploads";

#[get("/image/{uuid}")]
async fn get_image(uuid: web::Path<String>, req: HttpRequest) -> impl Responder {
    let filename = uuid.into_inner();
    if Uuid::parse_str(&filename).is_err() {
        return HttpResponse::BadRequest().body("Invalid UUID");
    }
    let path = PathBuf::from(UPLOAD_DIR).join(&filename);

    if !path.exists() {
        return HttpResponse::NotFound().body("Image not found");
    }

    match NamedFile::open(path) {
        Ok(file) => file.use_last_modified(true).into_response(&req),
        Err(_) => HttpResponse::InternalServerError().body("Failed to read image"),
    }
}

#[derive(Debug, MultipartForm)]
struct UploadForm {
    file: TempFile,
}

#[post("/image/upload")]
async fn upload_image(MultipartForm(form): MultipartForm<UploadForm>) -> impl Responder {
    std::fs::create_dir_all(UPLOAD_DIR).ok();
    let p = form.file.file.path();
    let uuid = Uuid::new_v4().to_string();
    let to = PathBuf::from(UPLOAD_DIR).join(&uuid);
    if let Err(err) = std::fs::rename(p, to) {
        return HttpResponse::InternalServerError().body(format!("Failed to rename file: {}", err));
    }
    HttpResponse::Ok().body(uuid)
}

#[derive(Debug, MultipartForm)]
struct TranslateForm {
    image: TempFile,
    settings: Option<MpBytes>,
    save_mask: Option<MpBytes>,
}

#[post("/translate")]
async fn translate(
    state: web::Data<AppState>,
    MultipartForm(form): MultipartForm<TranslateForm>,
) -> impl Responder {
    if form.image.size == 0 {
        return fail(StatusCode::BAD_REQUEST, "没有选择图片");
    }

    let settings = match parse_settings(&field_text(form.settings.as_ref())) {
        Ok(s) => Arc::new(s),
        Err(e) => return fail(StatusCode::BAD_REQUEST, e),
    };
    let save_mask = field_text(form.save_mask.as_ref()).trim() == "1";

    let bytes = match fs::read(form.image.file.path()) {
        Ok(b) => b,
        Err(e) => {
            return fail(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("读取上传图片失败: {e}"),
            );
        }
    };
    let img = match image::load_from_memory(&bytes) {
        Ok(i) => i,
        Err(e) => return fail(StatusCode::BAD_REQUEST, format!("无法解码图片: {e}")),
    };

    let tmp = PathBuf::from(std::env::temp_dir()).join(format!("mit-api-{}", Uuid::new_v4()));
    if let Err(e) = fs::create_dir_all(&tmp) {
        return fail(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("创建临时目录失败: {e}"),
        );
    }
    let _guard = TmpDir(tmp.clone());
    let ext = settings.render.renderer.extension();
    let out_path = tmp.join(format!("out.{ext}"));
    let mask_path = save_mask.then(|| tmp.join("out.mask.png"));

    let wall = Instant::now();
    state.inflight.fetch_add(1, Ordering::Relaxed);
    let _inflight = Inflight(&state.inflight);
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    if state
        .jobs
        .send(TranslateJob {
            img,
            settings: settings.clone(),
            mask_out: mask_path.clone(),
            reply: reply_tx,
        })
        .await
        .is_err()
    {
        return fail(StatusCode::INTERNAL_SERVER_ERROR, "服务内部任务已退出");
    }
    let exp = match reply_rx.await {
        Ok(Ok(Some(e))) => e,
        Ok(Ok(None)) => {
            return fail(StatusCode::UNPROCESSABLE_ENTITY, "未检测到可翻译内容");
        }
        Ok(Err(e)) => {
            return fail(StatusCode::INTERNAL_SERVER_ERROR, format!("执行失败: {e}"));
        }
        Err(_) => return fail(StatusCode::INTERNAL_SERVER_ERROR, "服务内部任务已退出"),
    };

    let ocr: Vec<serde_json::Value> = exp
        .blocks
        .iter()
        .map(|b| {
            serde_json::json!({
                "text": b.text,
                "translation": b.translation(),
            })
        })
        .collect();
    eprintln!(
        "OCR_JSON {}",
        serde_json::to_string(&ocr).unwrap_or_else(|_| "[]".into())
    );

    let render_t = Instant::now();
    if let Err(e) = write_render(exp, &settings, &out_path) {
        return fail(StatusCode::INTERNAL_SERVER_ERROR, e);
    }
    eprintln!("PERF render {}", render_t.elapsed().as_millis());
    let wall_ms = wall.elapsed().as_millis() as u64;
    let _ = std::io::stderr().flush();

    let data = match fs::read(&out_path) {
        Ok(d) => b64_encode(&d),
        Err(e) => {
            return fail(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("读取渲染结果失败: {e}"),
            );
        }
    };
    let (mask, mask_mime) = match &mask_path {
        Some(p) if p.exists() => match fs::read(p) {
            Ok(d) => (
                serde_json::Value::String(b64_encode(&d)),
                serde_json::Value::String("image/png".into()),
            ),
            Err(e) => {
                return fail(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("读取 mask 失败: {e}"),
                );
            }
        },
        _ => (serde_json::Value::Null, serde_json::Value::Null),
    };

    HttpResponse::Ok().json(serde_json::json!({
        "ok": true,
        "mime": mime_of(&settings.render.renderer),
        "filename": format!("out.{ext}"),
        "data": data,
        "mask": mask,
        "mask_mime": mask_mime,
        "ocr": ocr,
        "wall_ms": wall_ms,
    }))
}

pub async fn main(models: Models, host: &str, port: u16) -> std::io::Result<()> {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<TranslateJob>(1);
    tokio::spawn(async move {
        let mut models = models;
        while let Some(job) = rx.recv().await {
            let result = models
                .execute(job.img, &job.settings, None, job.mask_out)
                .await;
            let _ = job.reply.send(result);
        }
    });
    let state = web::Data::new(AppState {
        jobs: tx,
        started: Instant::now(),
        inflight: AtomicUsize::new(0),
    });
    HttpServer::new(move || {
        App::new()
            .app_data(state.clone())
            .app_data(web::PayloadConfig::new(64 * 1024 * 1024))
            .app_data(
                MultipartFormConfig::default()
                    .total_limit(64 * 1024 * 1024)
                    .memory_limit(4 * 1024 * 1024),
            )
            .service(health)
            .service(translate)
            .service(defaults_detector)
            .service(defaults_ocr)
            .service(defaults_mask_refinement)
            .service(defaults_translator)
            .service(defaults_inpainter)
            .service(defaults_colorizer)
            .service(defaults_render)
            .service(upload_image)
            .service(get_image)
            .service(hello)
    })
    .workers(1)
    .bind((host, port))?
    .run()
    .await
}
