//! Daemon 托管的 Web 版: gui/dist 在编译期嵌进二进制 (见 build.rs), 整个站点挂在 `/`。
//!
//! 它是 actix 的 default_service: 只有所有 API 路由都没匹配时才会走到这里, 所以静态文件永远遮不住 API,
//! 也不需要鉴权白名单。页面是匿名可开的, 鉴权只发生在它调用的 API 上。同源部署, 不加 CORS。

use actix_web::http::header::{
    HeaderValue, ACCEPT_ENCODING, CACHE_CONTROL, CONTENT_ENCODING, CONTENT_TYPE, VARY,
};
use actix_web::http::Method;
use actix_web::{HttpRequest, HttpResponse};

pub struct EmbeddedFile {
    pub path: &'static str,
    pub identity: &'static [u8],
    pub br: Option<&'static [u8]>,
    pub gzip: Option<&'static [u8]>,
}

// 按 path 升序, 由 build.rs 生成。
include!(concat!(env!("OUT_DIR"), "/site.rs"));

#[cfg(test)]
pub fn files() -> &'static [EmbeddedFile] {
    FILES
}

fn lookup(path: &str) -> Option<&'static EmbeddedFile> {
    FILES
        .binary_search_by_key(&path, |f| f.path)
        .ok()
        .map(|i| &FILES[i])
}

fn content_type(path: &str) -> &'static str {
    match path.rsplit_once('.').map(|(_, ext)| ext) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("webmanifest") => "application/manifest+json",
        Some("txt") => "text/plain; charset=utf-8",
        // 浏览器只对 application/wasm 做流式编译。
        Some("wasm") => "application/wasm",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        _ => "application/octet-stream",
    }
}

/// 文件名 (或所在目录名) 里带 NSH 的 10 位十六进制摘要, 内容变了名字就变, 可以永久缓存。
fn is_digested(path: &str) -> bool {
    path.split('/').any(|segment| {
        segment.split('.').skip(1).any(|part| {
            part.len() == 10 && part.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        })
    })
}

fn cache_control(path: &str) -> &'static str {
    if is_digested(path) {
        "public, max-age=31536000, immutable"
    } else {
        // 入口页和 manifest 的名字固定, 每次都要向 Daemon 确认, 升级后才不会拿到旧页面。
        "no-cache"
    }
}

/// Accept-Encoding 里 `coding` 是否被接受 (q=0 视为拒绝)。
fn accepts(header: &str, coding: &str) -> bool {
    header.split(',').any(|entry| {
        let mut parts = entry.split(';');
        let name = parts.next().unwrap_or("").trim();
        name.eq_ignore_ascii_case(coding)
            && parts.all(|param| {
                let param = param.trim();
                match param
                    .strip_prefix("q=")
                    .or_else(|| param.strip_prefix("Q="))
                {
                    Some(q) => q.trim().parse::<f32>().is_ok_and(|q| q > 0.0),
                    None => true,
                }
            })
    })
}

fn not_found() -> HttpResponse {
    HttpResponse::NotFound().json(serde_json::json!({ "ok": false, "error": "not found" }))
}

pub async fn serve(req: HttpRequest) -> HttpResponse {
    if !matches!(*req.method(), Method::GET | Method::HEAD) {
        return not_found();
    }
    let path = match req.path() {
        "/" => "index.html",
        other => other.trim_start_matches('/'),
    };
    let Some(file) = lookup(path) else {
        return not_found();
    };
    let accept = req
        .headers()
        .get(ACCEPT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let (body, encoding) = match (file.br, file.gzip) {
        (Some(br), _) if accepts(accept, "br") => (br, Some("br")),
        (_, Some(gz)) if accepts(accept, "gzip") => (gz, Some("gzip")),
        _ => (file.identity, None),
    };
    let mut res = HttpResponse::Ok();
    res.insert_header((CONTENT_TYPE, content_type(path)))
        .insert_header((CACHE_CONTROL, cache_control(path)));
    if file.br.is_some() || file.gzip.is_some() {
        res.insert_header((VARY, HeaderValue::from_static("Accept-Encoding")));
    }
    if let Some(encoding) = encoding {
        res.insert_header((CONTENT_ENCODING, encoding));
    }
    res.body(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_files_are_sorted_for_binary_search() {
        assert!(FILES.windows(2).all(|w| w[0].path < w[1].path));
        assert!(lookup("index.html").is_some());
    }

    #[test]
    fn digest_is_recognised_in_file_and_directory_names() {
        assert!(is_digested("app.8b16b001bb.js"));
        assert!(is_digested("facades/nsh-rt.4460431aa5.js"));
        assert!(is_digested("browser.9488a4b83e/loader.js"));
        assert!(!is_digested("index.html"));
        assert!(!is_digested("manifest.webmanifest"));
        assert!(!is_digested("browser.9488a4b83/loader.js"));
        assert!(!is_digested("8b16b001bb.js"));
    }

    #[test]
    fn accept_encoding_honours_q_zero() {
        assert!(accepts("gzip, deflate, br", "br"));
        assert!(accepts("br;q=0.5", "br"));
        assert!(!accepts("br;q=0", "br"));
        assert!(!accepts("gzip", "br"));
        assert!(!accepts("", "gzip"));
    }
}
