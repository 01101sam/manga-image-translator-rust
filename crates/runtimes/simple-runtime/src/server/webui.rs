use actix_web::{get, HttpResponse};

const INDEX_HTML: &str = include_str!("../../../../../gui/index.html");

#[get("/")]
pub async fn index() -> HttpResponse {
    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(INDEX_HTML)
}
