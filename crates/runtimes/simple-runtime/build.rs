//! 把 gui/dist 里的每个文件连同它的 .br / .gz 预压缩版本生成成 include_bytes! 表, 编进二进制。
//! 站点由 scripts/build-ui.sh web 生成且不入库, 缺失时在这里给出明确的构建错误。

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::{env, fs, process};

fn collect(dir: &Path, root: &Path, out: &mut BTreeSet<String>) {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("读取 {} 失败: {e}", dir.display()))
        .map(|e| e.expect("读取目录项失败").path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect(&path, root, out);
        } else {
            let rel = path.strip_prefix(root).expect("文件应在站点目录下");
            let parts: Vec<_> = rel.iter().map(|p| p.to_string_lossy()).collect();
            out.insert(parts.join("/"));
        }
    }
}

fn embed_site() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let dist = manifest.join("../../../gui/dist");
    // 对目录声明 rerun-if-changed 时 cargo 会递归检查其中文件的修改时间; 目录不存在时每次都重跑。
    println!("cargo:rerun-if-changed={}", dist.display());
    println!("cargo:rerun-if-changed=build.rs");

    if !dist.join("index.html").is_file() {
        eprintln!(
            "\nerror: 缺少 {}/index.html。Daemon 把 Web 版编进二进制, 请先运行 scripts/build-ui.sh web 生成 gui/dist。\n",
            dist.display()
        );
        process::exit(1);
    }
    let dist = dist.canonicalize().expect("canonicalize gui/dist");

    let mut all = BTreeSet::new();
    collect(&dist, &dist, &mut all);

    let mut code = String::from("static FILES: &[EmbeddedFile] = &[\n");
    for path in &all {
        let sibling = |ext: &str| all.contains(&format!("{path}.{ext}"));
        // x.js.br 只作为 x.js 的压缩副本, 不单独对外。
        let is_variant = |ext: &str| {
            path.strip_suffix(&format!(".{ext}"))
                .is_some_and(|base| all.contains(base))
        };
        if is_variant("br") || is_variant("gz") {
            continue;
        }
        let include =
            |rel: &str| format!("include_bytes!({:?})", dist.join(rel).display().to_string());
        let variant = |ext: &str| {
            if sibling(ext) {
                format!("Some({})", include(&format!("{path}.{ext}")))
            } else {
                "None".to_owned()
            }
        };
        writeln!(
            code,
            "    EmbeddedFile {{ path: {path:?}, identity: {}, br: {}, gzip: {} }},",
            include(path),
            variant("br"),
            variant("gz"),
        )
        .unwrap();
    }
    code.push_str("];\n");

    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR")).join("site.rs");
    fs::write(out, code).expect("写入 site.rs 失败");
}
/// 把仓库根 .env 里的 DEEPSEEK_API_KEY 编进二进制。
fn embed_deepseek_key() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let env_path = manifest_dir.join("../../..").join(".env");
    if !env_path.exists() {
        return;
    }
    println!("cargo:rerun-if-changed={}", env_path.display());
    let Ok(contents) = fs::read_to_string(&env_path) else {
        return;
    };
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key == "DEEPSEEK_API_KEY" && !value.is_empty() {
            println!("cargo:rustc-env=DEEPSEEK_API_KEY={value}");
        }
    }
}
fn main() {
    embed_deepseek_key();
    embed_site();
}
