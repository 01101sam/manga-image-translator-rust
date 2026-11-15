use actix_web::{get, HttpResponse};

const INDEX_HTML: &str = include_str!("../../../../../gui/index.html");

fn page() -> HttpResponse {
    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(INDEX_HTML)
}

/// `/` 与 `/index.html` 免鉴权：页面本身匿名可开，鉴权只发生在它调用的 API 上。
#[get("/")]
pub async fn index() -> HttpResponse {
    page()
}

#[get("/index.html")]
pub async fn index_html() -> HttpResponse {
    page()
}

/// NSH web bundle 的静态资源前缀，同样免鉴权。bundle 嵌进来之前没有任何资源，一律 404，
/// 先占住前缀，以后加资源不会再碰鉴权。
#[get("/assets/{path:.*}")]
pub async fn asset() -> HttpResponse {
    HttpResponse::NotFound().json(serde_json::json!({ "ok": false, "error": "asset not found" }))
}
