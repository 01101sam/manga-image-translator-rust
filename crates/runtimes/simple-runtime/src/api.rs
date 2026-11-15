use std::future::{ready, Ready};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use actix_multipart::form::{
    bytes::Bytes as MpBytes,
    tempfile::{TempFile, TempFileConfig},
    MultipartForm, MultipartFormConfig,
};
use actix_multipart::MultipartError;
use actix_web::dev::Payload;
use actix_web::error::{InternalError, JsonPayloadError, PayloadError};
use actix_web::http::StatusCode;
use actix_web::{
    get, post, put,
    web::{self},
    App, Error, FromRequest, HttpRequest, HttpResponse, HttpServer, Responder,
};
use serde::Deserialize;
use tokio_util::io::ReaderStream;

use crate::server::artifacts::ArtifactStore;
use crate::server::bonjour::Bonjour;
use crate::server::config::{ConfigError, ConfigStore, DaemonConfig};
use crate::server::engine::{mime_of, Engine, WorkerParams};
use crate::server::jobs::{apply_overrides, CancelError, JobSpec, JobState, JobStore};
use crate::server::pairing::{key_matches, TokenStore};
use crate::server::preview::{self, Previews};
use crate::server::queue::Queue;
use crate::server::source::{self, ProbeError};
use crate::server::webui;

/// 一次上传（含 multipart 开销）的字节上限。图片落盘不占内存，所以这里只防磁盘被灌满。
const MAX_UPLOAD_BYTES: usize = 256 * 1024 * 1024;

pub struct DaemonArgs {
    pub host: String,
    pub port: u16,
    pub bonjour: bool,
    pub max_batch_size_ocr: usize,
    pub max_batch_size_upscaler: usize,
    pub cuda: bool,
}

pub struct AppState {
    engine: tokio::sync::Mutex<Engine>,
    jobs: Arc<JobStore>,
    queue: Arc<Queue>,
    config: tokio::sync::Mutex<ConfigStore>,
    tokens: Arc<TokenStore>,
    artifacts: Arc<ArtifactStore>,
    previews: Previews,
    max_upload_bytes: usize,
    started: Instant,
    batch_ocr: usize,
    batch_upscaler: usize,
    cuda: bool,
}

impl AppState {
    fn new(
        config: ConfigStore,
        artifacts: Arc<ArtifactStore>,
        tokens: Arc<TokenStore>,
        runtime: tokio::runtime::Handle,
        args: &DaemonArgs,
    ) -> Self {
        let queue = Arc::new(Queue::new());
        let jobs = Arc::new(JobStore::new());
        let engine = Engine::new(queue.clone(), jobs.clone(), artifacts.clone(), runtime);
        Self {
            engine: tokio::sync::Mutex::new(engine),
            jobs,
            queue,
            config: tokio::sync::Mutex::new(config),
            tokens,
            artifacts,
            previews: Previews::new(preview::MAX_CONCURRENT),
            max_upload_bytes: MAX_UPLOAD_BYTES,
            started: Instant::now(),
            batch_ocr: args.max_batch_size_ocr,
            batch_upscaler: args.max_batch_size_upscaler,
            cuda: args.cuda,
        }
    }
}

struct Authed;

/// 图片类 GET 路由额外接受 `?token=`：NSH 的 Image 和浏览器的 <img> 发不出 Authorization 头。
struct AuthedMedia;

#[derive(Deserialize)]
struct TokenQuery {
    token: String,
}

fn authorize(req: &HttpRequest, token: Option<String>) -> Result<(), Error> {
    let allowed = req
        .app_data::<web::Data<AppState>>()
        .zip(token)
        .is_some_and(|(state, token)| state.tokens.contains(&token));
    if allowed {
        Ok(())
    } else {
        Err(InternalError::from_response(
            "unauthorized",
            fail(StatusCode::UNAUTHORIZED, "unauthorized"),
        )
        .into())
    }
}

impl FromRequest for Authed {
    type Error = Error;
    type Future = Ready<Result<Self, Error>>;

    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        ready(authorize(req, bearer_token(req)).map(|()| Authed))
    }
}

impl FromRequest for AuthedMedia {
    type Error = Error;
    type Future = Ready<Result<Self, Error>>;

    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        let token = bearer_token(req).or_else(|| query_token(req));
        ready(authorize(req, token).map(|()| AuthedMedia))
    }
}

fn query_token(req: &HttpRequest) -> Option<String> {
    web::Query::<TokenQuery>::from_query(req.query_string())
        .ok()
        .map(|q| q.into_inner().token)
}

fn bearer_token(req: &HttpRequest) -> Option<String> {
    let raw = req
        .headers()
        .get(actix_web::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    let token = raw.strip_prefix("Bearer ").or_else(|| raw.strip_prefix("bearer "))?;
    let token = token.trim();
    if token.is_empty() {
        None
    } else {
        Some(token.to_owned())
    }
}

fn fail(status: StatusCode, error: impl Into<String>) -> HttpResponse {
    HttpResponse::build(status).json(serde_json::json!({
        "ok": false,
        "error": error.into(),
    }))
}

fn field_text(field: Option<&MpBytes>) -> String {
    field
        .map(|b| String::from_utf8_lossy(&b.data).into_owned())
        .unwrap_or_default()
}

fn engine_status_json(state: crate::server::engine::EngineState) -> HttpResponse {
    HttpResponse::Ok().json(serde_json::json!({ "state": state }))
}

#[get("/health")]
async fn health(state: web::Data<AppState>) -> impl Responder {
    let engine = state.engine.lock().await.state();
    HttpResponse::Ok().json(serde_json::json!({
        "ok": true,
        "service": "simple-runtime-api",
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_s": state.started.elapsed().as_secs(),
        "engine": engine,
    }))
}

#[derive(Deserialize)]
struct PairBody {
    key: String,
}

#[post("/pair")]
async fn pair(state: web::Data<AppState>, body: web::Json<PairBody>) -> impl Responder {
    // 每次请求现读 Config，保证 PUT /config 改掉 Pairing Key 后立即生效。
    let expected = state.config.lock().await.get().pairing_key.clone();
    if !key_matches(&expected, &body.key) {
        return fail(StatusCode::UNAUTHORIZED, "Pairing Key 不正确");
    }
    match state.tokens.issue() {
        Ok(token) => HttpResponse::Ok().json(serde_json::json!({ "token": token })),
        Err(e) => fail(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("保存 Token 失败: {e}"),
        ),
    }
}

#[get("/engine/status")]
async fn engine_status(state: web::Data<AppState>, _auth: Authed) -> impl Responder {
    engine_status_json(state.engine.lock().await.state())
}

#[post("/engine/start")]
async fn engine_start(state: web::Data<AppState>, _auth: Authed) -> impl Responder {
    let n = {
        let cfg = state.config.lock().await;
        cfg.get().apply_secrets();
        cfg.get().workers
    };
    let mut engine = state.engine.lock().await;
    engine
        .start(
            n,
            WorkerParams {
                batch_ocr: state.batch_ocr,
                batch_upscaler: state.batch_upscaler,
                cuda: state.cuda,
            },
        )
        .await;
    engine_status_json(engine.state())
}

#[post("/engine/stop")]
async fn engine_stop(state: web::Data<AppState>, _auth: Authed) -> impl Responder {
    let mut engine = state.engine.lock().await;
    engine.stop().await;
    engine_status_json(engine.state())
}

#[post("/engine/restart")]
async fn engine_restart(state: web::Data<AppState>, _auth: Authed) -> impl Responder {
    let n = {
        let cfg = state.config.lock().await;
        cfg.get().apply_secrets();
        cfg.get().workers
    };
    let mut engine = state.engine.lock().await;
    engine
        .restart(
            n,
            WorkerParams {
                batch_ocr: state.batch_ocr,
                batch_upscaler: state.batch_upscaler,
                cuda: state.cuda,
            },
        )
        .await;
    engine_status_json(engine.state())
}

#[get("/config")]
async fn get_config(state: web::Data<AppState>, _auth: Authed) -> impl Responder {
    let cfg = state.config.lock().await;
    HttpResponse::Ok().json(cfg.get())
}

#[put("/config")]
async fn put_config(
    state: web::Data<AppState>,
    _auth: Authed,
    body: web::Json<serde_json::Value>,
) -> impl Responder {
    let cfg: DaemonConfig = match serde_json::from_value(body.into_inner()) {
        Ok(c) => c,
        Err(e) => return fail(StatusCode::BAD_REQUEST, format!("invalid config: {e}")),
    };
    let mut store = state.config.lock().await;
    if let Err(e) = store.replace(cfg) {
        let status = match e {
            ConfigError::Invalid(_) => StatusCode::BAD_REQUEST,
            ConfigError::Io(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        return fail(status, e.message());
    }
    store.get().apply_secrets();
    HttpResponse::Ok().json(store.get())
}

#[derive(Debug, MultipartForm)]
struct JobForm {
    image: TempFile,
    overrides: Option<MpBytes>,
}

#[post("/jobs")]
async fn submit_job(
    state: web::Data<AppState>,
    _auth: Authed,
    MultipartForm(form): MultipartForm<JobForm>,
) -> impl Responder {
    if !state.engine.lock().await.state().accepts_jobs() {
        return fail(StatusCode::SERVICE_UNAVAILABLE, "engine is stopped");
    }
    if form.image.size == 0 {
        return fail(StatusCode::BAD_REQUEST, "没有选择图片");
    }
    let (base, downscale_long_edge) = {
        let store = state.config.lock().await;
        (
            store.get().settings_clone(),
            store.get().downscale_long_edge,
        )
    };
    let settings = match apply_overrides(&base, &field_text(form.overrides.as_ref())) {
        Ok(s) => s,
        Err(e) => return fail(StatusCode::BAD_REQUEST, e.message()),
    };
    let upload = form.image.file;
    let probe_path = upload.path().to_owned();
    let source_mime = match web::block(move || source::probe(&probe_path)).await {
        Ok(Ok(mime)) => mime,
        Ok(Err(e @ ProbeError::TooLarge { .. })) => {
            return fail(StatusCode::PAYLOAD_TOO_LARGE, e.message())
        }
        Ok(Err(e)) => return fail(StatusCode::BAD_REQUEST, e.message()),
        Err(e) => {
            return fail(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("读取上传图片失败: {e}"),
            )
        }
    };
    let job_id = JobStore::new_id();
    let source = state.artifacts.source_path_for(&job_id);
    if let Err(e) = upload.persist(&source) {
        return fail(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("保存上传图片失败: {e}"),
        );
    }
    state.jobs.create(
        job_id.clone(),
        JobSpec {
            source,
            source_mime,
            settings,
            downscale_long_edge,
        },
    );
    if let Err(full) = state.queue.try_push(job_id.clone()) {
        state.jobs.discard(&job_id);
        return fail(
            StatusCode::TOO_MANY_REQUESTS,
            format!("queue is full (capacity {})", full.capacity),
        );
    }
    HttpResponse::Ok().json(serde_json::json!({ "job_id": job_id }))
}

#[get("/jobs")]
async fn list_jobs(state: web::Data<AppState>, _auth: Authed) -> impl Responder {
    let jobs: Vec<serde_json::Value> = state
        .jobs
        .list()
        .into_iter()
        .map(|j| {
            serde_json::json!({
                "job_id": j.job_id,
                "state": j.state,
            })
        })
        .collect();
    HttpResponse::Ok().json(serde_json::json!({ "jobs": jobs }))
}

#[get("/jobs/{id}")]
async fn get_job(
    state: web::Data<AppState>,
    _auth: Authed,
    id: web::Path<String>,
) -> impl Responder {
    let id = id.into_inner();
    let Some(job) = state.jobs.get(&id) else {
        return fail(StatusCode::NOT_FOUND, "job not found");
    };
    let position_in_queue = if job.state == JobState::Queued {
        state.queue.position(&id).map(|i| i + 1)
    } else {
        None
    };
    HttpResponse::Ok().json(serde_json::json!({
        "state": job.state,
        "position_in_queue": position_in_queue,
        "error": job.error,
    }))
}

#[post("/jobs/{id}/cancel")]
async fn cancel_job(
    state: web::Data<AppState>,
    _auth: Authed,
    id: web::Path<String>,
) -> impl Responder {
    let id = id.into_inner();
    match state.jobs.cancel(&id) {
        Ok(state_now) => {
            state.queue.remove(&id);
            HttpResponse::Ok().json(serde_json::json!({ "state": state_now }))
        }
        Err(CancelError::NotFound) => fail(StatusCode::NOT_FOUND, "job not found"),
        Err(CancelError::NotQueued { state }) => fail(
            StatusCode::CONFLICT,
            format!("job is {state:?}"),
        ),
    }
}

/// 流式返回文件，大图不会整份读进内存。
async fn stream_file(path: &Path, mime: &str) -> HttpResponse {
    let file = match tokio::fs::File::open(path).await {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return fail(StatusCode::NOT_FOUND, "文件已被清理")
        }
        Err(e) => {
            return fail(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("读取文件失败: {e}"),
            )
        }
    };
    let len = match file.metadata().await {
        Ok(m) => m.len(),
        Err(e) => {
            return fail(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("读取文件失败: {e}"),
            )
        }
    };
    HttpResponse::Ok()
        .content_type(mime)
        // Job id 唯一，文件写完后不再变化，浏览器和 NSH Image 可以放心缓存。
        .insert_header(("Cache-Control", "private, max-age=86400, immutable"))
        .no_chunking(len)
        .streaming(ReaderStream::with_capacity(file, 256 * 1024))
}

#[derive(Deserialize)]
struct MediaQuery {
    w: Option<u32>,
}

/// 没有 `w` 时流式返回原文件；有 `w` 时返回缓存在实例目录里的降采样 JPEG。
/// NSH 的 Image 会把整个编码文件下载进内存再降采样，列表缩略图必须让 Daemon 先缩小。
async fn serve_media(
    state: &AppState,
    key: (&str, &str),
    path: &Path,
    mime: &str,
    width: Option<u32>,
) -> HttpResponse {
    let Some(width) = width else {
        return stream_file(path, mime).await;
    };
    if !preview::is_allowed(width) {
        return fail(
            StatusCode::BAD_REQUEST,
            format!("w 只能是 {:?} 之一", preview::WIDTHS),
        );
    }
    if !mime.starts_with("image/") {
        return fail(StatusCode::BAD_REQUEST, "这个文件不是图片，不能生成预览");
    }
    // 源文件被清理后，缓存里残留的预览也不再提供。
    if !tokio::fs::try_exists(path).await.unwrap_or(false) {
        return fail(StatusCode::NOT_FOUND, "文件已被清理");
    }
    let (id, kind) = key;
    let cache = state
        .artifacts
        .preview_path_for(&format!("{id}-{kind}-{width}"));
    let (from, to) = (path.to_owned(), cache.clone());
    match state
        .previews
        .ensure(&cache, move || preview::render(&from, &to, width))
        .await
    {
        Ok(()) => stream_file(&cache, "image/jpeg").await,
        Err(e) => fail(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("生成预览失败: {e}"),
        ),
    }
}

#[get("/jobs/{id}/artifact")]
async fn job_artifact(
    state: web::Data<AppState>,
    _auth: AuthedMedia,
    id: web::Path<String>,
    query: web::Query<MediaQuery>,
) -> impl Responder {
    let id = id.into_inner();
    let Some(job) = state.jobs.get(&id) else {
        return fail(StatusCode::NOT_FOUND, "job not found");
    };
    if job.state != JobState::Done {
        return fail(StatusCode::CONFLICT, "artifact not ready");
    }
    let Some(path) = job.artifact else {
        return fail(StatusCode::NOT_FOUND, "artifact missing");
    };
    let mime = job
        .mime
        .unwrap_or_else(|| mime_of(&job.renderer).to_string());
    serve_media(&state, (&id, "artifact"), &path, &mime, query.w).await
}

#[get("/jobs/{id}/source")]
async fn job_source(
    state: web::Data<AppState>,
    _auth: AuthedMedia,
    id: web::Path<String>,
    query: web::Query<MediaQuery>,
) -> impl Responder {
    let id = id.into_inner();
    let Some(job) = state.jobs.get(&id) else {
        return fail(StatusCode::NOT_FOUND, "job not found");
    };
    serve_media(
        &state,
        (&id, "source"),
        &job.source,
        job.source_mime,
        query.w,
    )
    .await
}

fn cleanup(artifacts: &ArtifactStore, bonjour: &Mutex<Option<Bonjour>>) {
    let _ = artifacts.remove_instance();
    if let Ok(mut g) = bonjour.lock() {
        if let Some(svc) = g.take() {
            svc.shutdown();
        }
    }
}

fn json_error(err: JsonPayloadError) -> Error {
    let status = match err {
        JsonPayloadError::Overflow { .. } | JsonPayloadError::OverflowKnownLength { .. } => {
            StatusCode::PAYLOAD_TOO_LARGE
        }
        _ => StatusCode::BAD_REQUEST,
    };
    let message = format!("请求体不是有效的 JSON: {err}");
    InternalError::from_response(err, fail(status, message)).into()
}

fn query_error(err: actix_web::error::QueryPayloadError) -> Error {
    let message = format!("查询参数无效: {err}");
    InternalError::from_response(err, fail(StatusCode::BAD_REQUEST, message)).into()
}

fn upload_error(err: MultipartError, max_bytes: usize) -> Error {
    let (status, message) = match &err {
        MultipartError::Payload(PayloadError::Overflow) => (
            StatusCode::PAYLOAD_TOO_LARGE,
            format!("上传的图片超过 {} MB 上限", max_bytes >> 20),
        ),
        _ => (StatusCode::BAD_REQUEST, format!("上传内容无法解析: {err}")),
    };
    InternalError::from_response(err, fail(status, message)).into()
}

fn configure(cfg: &mut web::ServiceConfig, state: web::Data<AppState>) {
    let max_upload = state.max_upload_bytes;
    let upload_dir = state.artifacts.upload_dir();
    cfg.app_data(state)
        .app_data(web::JsonConfig::default().error_handler(|err, _| json_error(err)))
        .app_data(web::QueryConfig::default().error_handler(|err, _| query_error(err)))
        .app_data(
            MultipartFormConfig::default()
                .total_limit(max_upload)
                .memory_limit(4 * 1024 * 1024)
                .error_handler(move |err, _| upload_error(err, max_upload)),
        )
        .app_data(TempFileConfig::default().directory(upload_dir))
        .service(webui::index)
        .service(webui::index_html)
        .service(webui::asset)
        .service(health)
        .service(pair)
        .service(engine_status)
        .service(engine_start)
        .service(engine_stop)
        .service(engine_restart)
        .service(get_config)
        .service(put_config)
        .service(submit_job)
        .service(list_jobs)
        .service(get_job)
        .service(cancel_job)
        .service(job_artifact)
        .service(job_source);
}

/// 等 Ctrl-C 或 SIGTERM。只收 SIGINT 的话，`kill` 掉的 Daemon 不会删实例目录，也不会注销 Bonjour。
async fn shutdown_signal() -> &'static str {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut term = signal(SignalKind::terminate()).expect("install SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => "ctrl-c",
            _ = term.recv() => "SIGTERM",
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
        "ctrl-c"
    }
}

pub async fn main(args: DaemonArgs) -> std::io::Result<()> {
    let artifacts = Arc::new(
        ArtifactStore::platform()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?,
    );
    let config = ConfigStore::platform()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.message()))?;
    let tokens = Arc::new(
        TokenStore::open(config.path())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?,
    );
    // Capture before HttpServer. actix handlers run on a current-thread runtime.
    let engine_rt = tokio::runtime::Handle::current();
    let state = web::Data::new(AppState::new(
        config,
        artifacts.clone(),
        tokens,
        engine_rt,
        &args,
    ));
    let bonjour = Mutex::new(if args.bonjour {
        match Bonjour::register(args.port) {
            Ok(svc) => Some(svc),
            Err(e) => {
                eprintln!("bonjour unavailable: {e}");
                None
            }
        }
    } else {
        None
    });

    let server = HttpServer::new({
        let state = state.clone();
        move || App::new().configure(|cfg| configure(cfg, state.clone()))
    })
    .bind((args.host.as_str(), args.port))?
    .run();
    let handle = server.handle();

    tokio::select! {
        r = server => {
            cleanup(&artifacts, &bonjour);
            r
        }
        signal = shutdown_signal() => {
            eprintln!("daemon received {signal}");
            state.engine.lock().await.stop().await;
            cleanup(&artifacts, &bonjour);
            handle.stop(true).await;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests;
