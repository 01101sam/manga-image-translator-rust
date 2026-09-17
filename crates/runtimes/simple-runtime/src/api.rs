use std::fs;
use std::future::{ready, Ready};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use actix_multipart::form::{
    bytes::Bytes as MpBytes, tempfile::TempFile, MultipartForm, MultipartFormConfig,
};
use actix_web::dev::Payload;
use actix_web::http::StatusCode;
use actix_web::{
    get, post, put,
    web::{self},
    App, Error, FromRequest, HttpRequest, HttpResponse, HttpServer, Responder,
};
use serde::Deserialize;

use crate::server::artifacts::ArtifactStore;
use crate::server::bonjour::Bonjour;
use crate::server::config::{ConfigStore, DaemonConfig};
use crate::server::engine::{mime_of, Engine, WorkerParams};
use crate::server::jobs::{apply_overrides, CancelError, JobState, JobStore};
use crate::server::pairing::{Pairing, SystemClock, TokenStore};
use crate::server::queue::Queue;
use crate::server::webui;

pub struct DaemonArgs {
    pub host: String,
    pub port: u16,
    pub max_batch_size_ocr: usize,
    pub max_batch_size_upscaler: usize,
    pub cuda: bool,
}

pub struct AppState {
    engine: tokio::sync::Mutex<Engine>,
    jobs: Arc<JobStore>,
    queue: Arc<Queue>,
    config: tokio::sync::Mutex<ConfigStore>,
    pairing: tokio::sync::Mutex<Pairing<SystemClock>>,
    tokens: Arc<TokenStore>,
    started: Instant,
    batch_ocr: usize,
    batch_upscaler: usize,
    cuda: bool,
}

struct Authed;

impl FromRequest for Authed {
    type Error = Error;
    type Future = Ready<Result<Self, Error>>;

    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        let allowed = req
            .app_data::<web::Data<AppState>>()
            .and_then(|state| bearer_token(req).filter(|t| state.tokens.contains(t)))
            .is_some();
        if allowed {
            ready(Ok(Authed))
        } else {
            ready(Err(actix_web::error::InternalError::from_response(
                "unauthorized",
                fail(StatusCode::UNAUTHORIZED, "unauthorized"),
            )
            .into()))
        }
    }
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

#[post("/pair/request")]
async fn pair_request(state: web::Data<AppState>) -> impl Responder {
    let pairing = state.pairing.lock().await;
    let _code = pairing.request();
    HttpResponse::Ok().json(serde_json::json!({ "ok": true }))
}

#[derive(Deserialize)]
struct ConfirmBody {
    code: String,
}

#[post("/pair/confirm")]
async fn pair_confirm(state: web::Data<AppState>, body: web::Json<ConfirmBody>) -> impl Responder {
    let pairing = state.pairing.lock().await;
    match pairing.confirm(body.code.trim()) {
        Ok(token) => HttpResponse::Ok().json(serde_json::json!({ "token": token })),
        Err(e) => fail(StatusCode::UNAUTHORIZED, e.message()),
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
    cfg.apply_secrets();
    let mut store = state.config.lock().await;
    if let Err(e) = store.replace(cfg) {
        return fail(StatusCode::INTERNAL_SERVER_ERROR, e.message());
    }
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
    let base = state.config.lock().await.get().settings_clone();
    let settings = match apply_overrides(&base, &field_text(form.overrides.as_ref())) {
        Ok(s) => s,
        Err(e) => return fail(StatusCode::BAD_REQUEST, e.message()),
    };
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
    let job_id = state.jobs.create(img, settings);
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

#[get("/jobs/{id}/artifact")]
async fn job_artifact(
    state: web::Data<AppState>,
    _auth: Authed,
    id: web::Path<String>,
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
    let data = match fs::read(&path) {
        Ok(d) => d,
        Err(e) => {
            return fail(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("读取 Artifact 失败: {e}"),
            );
        }
    };
    let mime = job
        .mime
        .unwrap_or_else(|| mime_of(&job.renderer).to_string());
    HttpResponse::Ok().content_type(mime).body(data)
}

fn cleanup(artifacts: &ArtifactStore, bonjour: &Mutex<Option<Bonjour>>) {
    let _ = artifacts.wipe();
    if let Ok(mut g) = bonjour.lock() {
        if let Some(svc) = g.take() {
            svc.shutdown();
        }
    }
}

pub async fn main(args: DaemonArgs) -> std::io::Result<()> {
    let artifacts = Arc::new(
        ArtifactStore::platform()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?,
    );
    let queue = Arc::new(Queue::new());
    let jobs = Arc::new(JobStore::new());
    let config = ConfigStore::platform()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.message()))?;
    let pairing = Pairing::open(config.path())
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    let tokens = pairing.tokens();
    // Capture before HttpServer. actix handlers run on a current-thread runtime.
    let engine_rt = tokio::runtime::Handle::current();
    let engine = Engine::new(
        queue.clone(),
        jobs.clone(),
        artifacts.clone(),
        engine_rt,
    );
    let bonjour = Mutex::new(match Bonjour::register(args.port) {
        Ok(svc) => Some(svc),
        Err(e) => {
            eprintln!("bonjour unavailable: {e}");
            None
        }
    });

    let state = web::Data::new(AppState {
        engine: tokio::sync::Mutex::new(engine),
        jobs,
        queue,
        config: tokio::sync::Mutex::new(config),
        pairing: tokio::sync::Mutex::new(pairing),
        tokens,
        started: Instant::now(),
        batch_ocr: args.max_batch_size_ocr,
        batch_upscaler: args.max_batch_size_upscaler,
        cuda: args.cuda,
    });

    let server = HttpServer::new({
        let state = state.clone();
        move || {
            App::new()
                .app_data(state.clone())
                .app_data(web::PayloadConfig::new(64 * 1024 * 1024))
                .app_data(
                    MultipartFormConfig::default()
                        .total_limit(64 * 1024 * 1024)
                        .memory_limit(4 * 1024 * 1024),
                )
                .service(webui::index)
                .service(health)
                .service(pair_request)
                .service(pair_confirm)
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
        }
    })
    .bind((args.host.as_str(), args.port))?
    .run();
    let handle = server.handle();

    tokio::select! {
        r = server => {
            cleanup(&artifacts, &bonjour);
            r
        }
        _ = tokio::signal::ctrl_c() => {
            eprintln!("daemon received ctrl-c");
            state.engine.lock().await.stop().await;
            cleanup(&artifacts, &bonjour);
            handle.stop(true).await;
            Ok(())
        }
    }
}
