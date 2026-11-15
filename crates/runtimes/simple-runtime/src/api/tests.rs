use std::fs;
use std::path::PathBuf;

use actix_web::{http::StatusCode, test, web, App};
use serde_json::{json, Value};

use super::*;
use crate::server::config::DEFAULT_PAIRING_KEY;

fn test_args() -> DaemonArgs {
    DaemonArgs {
        host: "127.0.0.1".into(),
        port: 0,
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
        let config = ConfigStore::open(self.config_dir()).unwrap();
        let artifacts = Arc::new(ArtifactStore::open_at(self.dir.join("artifacts")).unwrap());
        let tokens = Arc::new(TokenStore::open(config.path()).unwrap());
        web::Data::new(AppState::new(
            config,
            artifacts,
            tokens,
            tokio::runtime::Handle::current(),
            &test_args(),
        ))
    }
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
