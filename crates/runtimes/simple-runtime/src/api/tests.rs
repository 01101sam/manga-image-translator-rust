use std::fs;
use std::path::PathBuf;

use actix_web::{http::StatusCode, test, web, App};
use serde_json::{json, Value};

use super::*;
use crate::server::config::DEFAULT_PAIRING_KEY;
use crate::server::engine::WorkerParams;
use crate::server::source::test_support::{png_bytes, png_header_claiming};

fn test_args() -> DaemonArgs {
    DaemonArgs {
        host: "127.0.0.1".into(),
        port: 0,
        bonjour: false,
        max_batch_size_ocr: 1,
        max_batch_size_upscaler: 1,
        cuda: false,
    }
}

/// 每个用例独占一个临时根目录。绝不能走 platform()，它会清空真实 Daemon 的 Artifact 目录。
struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("it-api-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        Self { dir }
    }

    fn config_dir(&self) -> PathBuf {
        self.dir.join("config")
    }

    /// 同一个 Fixture 多次调用等价于重启 Daemon：内存状态全新，磁盘状态沿用。
    fn state(&self) -> web::Data<AppState> {
        self.state_with(|_| {})
    }

    fn state_with(&self, tweak: impl FnOnce(&mut AppState)) -> web::Data<AppState> {
        let config = ConfigStore::open(self.config_dir()).unwrap();
        let artifacts = Arc::new(ArtifactStore::open_at(self.dir.join("artifacts")).unwrap());
        let tokens = Arc::new(TokenStore::open(config.path()).unwrap());
        let mut state = AppState::new(
            config,
            artifacts,
            tokens,
            tokio::runtime::Handle::current(),
            &test_args(),
        );
        tweak(&mut state);
        web::Data::new(state)
    }
}

/// 引擎以 0 个 Worker 启动会直接进入 Running：Job 留在队列里，不需要加载任何模型。
async fn running_state(state: &web::Data<AppState>) -> String {
    let params = WorkerParams {
        batch_ocr: 1,
        batch_upscaler: 1,
        cuda: false,
    };
    state.engine.lock().await.start(0, params).await;
    state.tokens.issue().unwrap()
}

fn multipart(image: &[u8]) -> (String, Vec<u8>) {
    let boundary = "----it-boundary";
    let mut body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"image\"; filename=\"p.png\"\r\nContent-Type: image/png\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(image);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

macro_rules! app {
    ($state:expr) => {{
        let state = $state;
        test::init_service(App::new().configure(move |cfg| configure(cfg, state.clone()))).await
    }};
}

macro_rules! send {
    ($app:expr, $req:expr) => {{
        let resp = test::call_service(&$app, $req.to_request()).await;
        let status = resp.status();
        let body = test::read_body(resp).await;
        (
            status,
            serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null),
        )
    }};
}

macro_rules! submit {
    ($app:expr, $token:expr, $image:expr) => {{
        let (content_type, body) = multipart($image);
        send!(
            $app,
            test::TestRequest::post()
                .uri("/jobs")
                .insert_header(bearer($token))
                .insert_header(("Content-Type", content_type))
                .set_payload(body)
        )
    }};
}

macro_rules! fetch {
    ($app:expr, $uri:expr, $token:expr) => {{
        let mut req = test::TestRequest::get().uri($uri);
        if let Some(t) = $token {
            req = req.insert_header(bearer(t));
        }
        let resp = test::call_service(&$app, req.to_request()).await;
        let status = resp.status();
        let content_type = resp
            .headers()
            .get("content-type")
            .map(|v| v.to_str().unwrap().to_owned());
        let content_length = resp
            .headers()
            .get("content-length")
            .map(|v| v.to_str().unwrap().to_owned());
        let body = test::read_body(resp).await;
        (status, content_type, content_length, body)
    }};
}

macro_rules! pair_with {
    ($app:expr, $key:expr) => {
        send!(
            $app,
            test::TestRequest::post()
                .uri("/pair")
                .set_json(json!({ "key": $key }))
        )
    };
}

fn bearer(token: &str) -> (&'static str, String) {
    ("Authorization", format!("Bearer {token}"))
}

#[actix_web::test]
async fn correct_key_issues_a_token_that_unlocks_jobs() {
    let fx = Fixture::new();
    let app = app!(fx.state());

    let (status, body) = pair_with!(app, DEFAULT_PAIRING_KEY);
    assert_eq!(status, StatusCode::OK);
    let token = body["token"].as_str().expect("token in body").to_owned();

    let (status, body) = send!(
        app,
        test::TestRequest::get()
            .uri("/jobs")
            .insert_header(bearer(&token))
    );
    assert_eq!(status, StatusCode::OK);
    assert!(body["jobs"].is_array());

    let (status, _) = send!(app, test::TestRequest::get().uri("/jobs"));
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[actix_web::test]
async fn wrong_key_is_401_and_issues_no_token() {
    let fx = Fixture::new();
    let app = app!(fx.state());

    for wrong in ["", "sam01102", "sam0110", "SAM01101", " sam01101"] {
        let (status, body) = pair_with!(app, wrong);
        assert_eq!(status, StatusCode::UNAUTHORIZED, "key {wrong:?}");
        assert!(body["error"].is_string());
        assert!(body.get("token").is_none());
    }
    let tokens = fx.config_dir().join("tokens.json");
    assert!(
        !tokens.exists(),
        "a rejected key must not touch tokens.json"
    );
}

#[actix_web::test]
async fn pair_body_without_key_is_rejected() {
    let fx = Fixture::new();
    let app = app!(fx.state());
    let (status, _) = send!(
        app,
        test::TestRequest::post().uri("/pair").set_json(json!({}))
    );
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[actix_web::test]
async fn legacy_config_without_pairing_key_pairs_with_the_default() {
    let fx = Fixture::new();
    fs::create_dir_all(fx.config_dir()).unwrap();
    fs::write(
        fx.config_dir().join("config.json"),
        br#"{"workers": 1, "api_key": "sk-legacy"}"#,
    )
    .unwrap();
    let app = app!(fx.state());

    let (status, _) = pair_with!(app, DEFAULT_PAIRING_KEY);
    assert_eq!(status, StatusCode::OK);
}

#[actix_web::test]
async fn token_survives_a_daemon_restart() {
    let fx = Fixture::new();
    let token = {
        let app = app!(fx.state());
        let (status, body) = pair_with!(app, DEFAULT_PAIRING_KEY);
        assert_eq!(status, StatusCode::OK);
        body["token"].as_str().unwrap().to_owned()
    };

    let app = app!(fx.state());
    let (status, _) = send!(
        app,
        test::TestRequest::get()
            .uri("/config")
            .insert_header(bearer(&token))
    );
    assert_eq!(status, StatusCode::OK);
}

#[actix_web::test]
async fn changing_the_key_keeps_issued_tokens_and_switches_future_pairing() {
    let fx = Fixture::new();
    let app = app!(fx.state());
    let (_, body) = pair_with!(app, DEFAULT_PAIRING_KEY);
    let token = body["token"].as_str().unwrap().to_owned();

    let (_, mut cfg) = send!(
        app,
        test::TestRequest::get()
            .uri("/config")
            .insert_header(bearer(&token))
    );
    assert_eq!(cfg["pairing_key"], DEFAULT_PAIRING_KEY);
    cfg["pairing_key"] = json!("new-key");
    let (status, saved) = send!(
        app,
        test::TestRequest::put()
            .uri("/config")
            .insert_header(bearer(&token))
            .set_json(&cfg)
    );
    assert_eq!(status, StatusCode::OK);
    assert_eq!(saved["pairing_key"], "new-key");

    let (status, _) = send!(
        app,
        test::TestRequest::get()
            .uri("/jobs")
            .insert_header(bearer(&token))
    );
    assert_eq!(status, StatusCode::OK, "tokens must outlive a key change");
    assert_eq!(
        pair_with!(app, DEFAULT_PAIRING_KEY).0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(pair_with!(app, "new-key").0, StatusCode::OK);
}

#[actix_web::test]
async fn blank_pairing_key_is_refused_and_the_old_key_still_works() {
    let fx = Fixture::new();
    let app = app!(fx.state());
    let (_, body) = pair_with!(app, DEFAULT_PAIRING_KEY);
    let token = body["token"].as_str().unwrap().to_owned();

    let (_, mut cfg) = send!(
        app,
        test::TestRequest::get()
            .uri("/config")
            .insert_header(bearer(&token))
    );
    cfg["pairing_key"] = json!("  ");
    let (status, body) = send!(
        app,
        test::TestRequest::put()
            .uri("/config")
            .insert_header(bearer(&token))
            .set_json(&cfg)
    );
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("Pairing Key"));
    assert_eq!(pair_with!(app, DEFAULT_PAIRING_KEY).0, StatusCode::OK);
}

#[actix_web::test]
async fn the_old_pairing_routes_are_gone() {
    let fx = Fixture::new();
    let app = app!(fx.state());
    for path in ["/pair/request", "/pair/confirm"] {
        let (status, _) = send!(app, test::TestRequest::post().uri(path));
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
    }
}

fn upload_files(fx: &Fixture) -> usize {
    fs::read_dir(
        fs::read_dir(fx.dir.join("artifacts"))
            .unwrap()
            .flatten()
            .next()
            .unwrap()
            .path()
            .join("src"),
    )
    .unwrap()
    .count()
}

#[actix_web::test]
async fn oversized_images_are_413_from_the_header_and_nothing_is_kept() {
    let fx = Fixture::new();
    let state = fx.state();
    let token = running_state(&state).await;
    let app = app!(state);

    for (w, h) in [(20_000, 20_000), (800, 100_000)] {
        let (status, body) = submit!(app, &token, &png_header_claiming(w, h));
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{w}x{h}");
        let error = body["error"].as_str().unwrap();
        assert!(
            error.contains(&format!("{w}x{h}")) && error.contains("太大"),
            "{error}"
        );
    }
    let (_, jobs) = send!(
        app,
        test::TestRequest::get()
            .uri("/jobs")
            .insert_header(bearer(&token))
    );
    assert_eq!(jobs["jobs"].as_array().unwrap().len(), 0);
    assert_eq!(
        upload_files(&fx),
        0,
        "rejected uploads must not stay on disk"
    );
}

#[actix_web::test]
async fn non_images_are_400() {
    let fx = Fixture::new();
    let state = fx.state();
    let token = running_state(&state).await;
    let app = app!(state);
    let (status, body) = submit!(app, &token, b"this is not a picture");
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("无法识别图片"));
    assert_eq!(upload_files(&fx), 0);
}

#[actix_web::test]
async fn an_upload_over_the_limit_is_413_with_a_json_error() {
    let fx = Fixture::new();
    let state = fx.state_with(|s| s.max_upload_bytes = 4096);
    let token = running_state(&state).await;
    let app = app!(state);
    let (status, body) = submit!(app, &token, &vec![7u8; 64 * 1024]);
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(body["ok"], false);
    assert!(body["error"].as_str().unwrap().contains("上限"), "{body}");
    assert_eq!(upload_files(&fx), 0);
}

#[actix_web::test]
async fn six_queued_jobs_hold_paths_not_pixels() {
    let fx = Fixture::new();
    let state = fx.state();
    let token = running_state(&state).await;
    let app = app!(state.clone());

    let image = png_bytes(64, 48);
    let mut ids = Vec::new();
    for _ in 0..6 {
        let (status, body) = submit!(app, &token, &image);
        assert_eq!(status, StatusCode::OK);
        ids.push(body["job_id"].as_str().unwrap().to_owned());
    }
    assert_eq!(upload_files(&fx), 6);
    for id in &ids {
        let work = state.jobs.take_for_run(id).unwrap();
        assert_eq!(fs::read(&work.source).unwrap(), image);
        assert_eq!(work.downscale_long_edge, Some(4096));
    }
}

#[actix_web::test]
async fn a_full_queue_rejects_and_removes_the_extra_upload() {
    let fx = Fixture::new();
    let state = fx.state();
    let token = running_state(&state).await;
    let app = app!(state);
    let image = png_bytes(8, 8);
    for _ in 0..crate::server::queue::QUEUE_CAPACITY {
        assert_eq!(submit!(app, &token, &image).0, StatusCode::OK);
    }
    let (status, _) = submit!(app, &token, &image);
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(upload_files(&fx), crate::server::queue::QUEUE_CAPACITY);
}

#[actix_web::test]
async fn source_and_artifact_are_streamed_with_their_length() {
    let fx = Fixture::new();
    let state = fx.state();
    let token = running_state(&state).await;
    let app = app!(state.clone());
    let image = png_bytes(32, 32);
    let (_, body) = submit!(app, &token, &image);
    let id = body["job_id"].as_str().unwrap().to_owned();

    let (status, content_type, length, bytes) =
        fetch!(app, &format!("/jobs/{id}/source"), Some(&token));
    assert_eq!(status, StatusCode::OK);
    assert_eq!(content_type.as_deref(), Some("image/png"));
    assert_eq!(length, Some(image.len().to_string()));
    assert_eq!(bytes.as_ref(), image.as_slice());

    let (status, ..) = fetch!(app, &format!("/jobs/{id}/artifact"), Some(&token));
    assert_eq!(status, StatusCode::CONFLICT, "not done yet");

    let artifact = state.artifacts.path_for(&id, "png");
    let rendered = png_bytes(16, 16);
    fs::write(&artifact, &rendered).unwrap();
    state.jobs.take_for_run(&id).unwrap();
    state
        .jobs
        .finish_ok(&id, artifact, "image/png".into())
        .unwrap();
    let (status, content_type, length, bytes) =
        fetch!(app, &format!("/jobs/{id}/artifact"), Some(&token));
    assert_eq!(status, StatusCode::OK);
    assert_eq!(content_type.as_deref(), Some("image/png"));
    assert_eq!(length, Some(rendered.len().to_string()));
    assert_eq!(bytes.as_ref(), rendered.as_slice());
}

#[actix_web::test]
async fn a_cancelled_jobs_source_is_gone() {
    let fx = Fixture::new();
    let state = fx.state();
    let token = running_state(&state).await;
    let app = app!(state);
    let (_, body) = submit!(app, &token, &png_bytes(8, 8));
    let id = body["job_id"].as_str().unwrap().to_owned();
    let (status, _) = send!(
        app,
        test::TestRequest::post()
            .uri(&format!("/jobs/{id}/cancel"))
            .insert_header(bearer(&token))
    );
    assert_eq!(status, StatusCode::OK);
    let (status, ..) = fetch!(app, &format!("/jobs/{id}/source"), Some(&token));
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[actix_web::test]
async fn query_token_unlocks_the_image_routes_only() {
    let fx = Fixture::new();
    let state = fx.state();
    let token = running_state(&state).await;
    let app = app!(state.clone());
    let (_, body) = submit!(app, &token, &png_bytes(8, 8));
    let id = body["job_id"].as_str().unwrap().to_owned();
    let artifact = state.artifacts.path_for(&id, "png");
    fs::write(&artifact, png_bytes(4, 4)).unwrap();
    state.jobs.take_for_run(&id).unwrap();
    state
        .jobs
        .finish_ok(&id, artifact, "image/png".into())
        .unwrap();

    for route in ["artifact", "source"] {
        let ok = format!("/jobs/{id}/{route}?token={token}");
        assert_eq!(
            fetch!(app, &ok, None::<&String>).0,
            StatusCode::OK,
            "{route}"
        );
        let missing = format!("/jobs/{id}/{route}");
        assert_eq!(
            fetch!(app, &missing, None::<&String>).0,
            StatusCode::UNAUTHORIZED
        );
        let wrong = format!("/jobs/{id}/{route}?token=nope");
        assert_eq!(
            fetch!(app, &wrong, None::<&String>).0,
            StatusCode::UNAUTHORIZED
        );
        let empty = format!("/jobs/{id}/{route}?token=");
        assert_eq!(
            fetch!(app, &empty, None::<&String>).0,
            StatusCode::UNAUTHORIZED
        );
    }
    for route in [
        format!("/jobs?token={token}"),
        format!("/jobs/{id}?token={token}"),
        format!("/config?token={token}"),
        format!("/engine/status?token={token}"),
    ] {
        assert_eq!(
            fetch!(app, &route, None::<&String>).0,
            StatusCode::UNAUTHORIZED,
            "{route} must not accept a query token"
        );
    }
}

#[actix_web::test]
async fn public_routes_need_no_token_and_every_api_route_does() {
    let fx = Fixture::new();
    let app = app!(fx.state());
    for path in [
        "/",
        "/index.html",
        "/health",
        "/assets/app.js",
        "/assets/chunks/a.css",
    ] {
        let status = fetch!(app, path, None::<&String>).0;
        assert_ne!(status, StatusCode::UNAUTHORIZED, "{path}");
    }
    assert_eq!(
        fetch!(app, "/index.html", None::<&String>).0,
        StatusCode::OK
    );
    assert_eq!(
        fetch!(app, "/assets/app.js", None::<&String>).0,
        StatusCode::NOT_FOUND,
        "no bundle is embedded yet"
    );
    for path in [
        "/engine/status",
        "/config",
        "/jobs",
        "/jobs/x",
        "/jobs/x/artifact",
        "/jobs/x/source",
    ] {
        assert_eq!(
            fetch!(app, path, None::<&String>).0,
            StatusCode::UNAUTHORIZED,
            "{path}"
        );
    }
}

#[actix_web::test]
async fn config_exposes_and_persists_the_downscale_setting() {
    let fx = Fixture::new();
    let app = app!(fx.state());
    let (_, body) = pair_with!(app, DEFAULT_PAIRING_KEY);
    let token = body["token"].as_str().unwrap().to_owned();
    let (_, mut cfg) = send!(
        app,
        test::TestRequest::get()
            .uri("/config")
            .insert_header(bearer(&token))
    );
    assert_eq!(cfg["downscale_long_edge"], 4096);
    cfg["downscale_long_edge"] = Value::Null;
    let (status, _) = send!(
        app,
        test::TestRequest::put()
            .uri("/config")
            .insert_header(bearer(&token))
            .set_json(&cfg)
    );
    assert_eq!(status, StatusCode::OK);

    let app = app!(fx.state());
    let (_, cfg) = send!(
        app,
        test::TestRequest::get()
            .uri("/config")
            .insert_header(bearer(&token))
    );
    assert_eq!(cfg["downscale_long_edge"], Value::Null);

    let mut bad = cfg.clone();
    bad["downscale_long_edge"] = json!(0);
    let (status, _) = send!(
        app,
        test::TestRequest::put()
            .uri("/config")
            .insert_header(bearer(&token))
            .set_json(&bad)
    );
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
