use std::fs;
use std::path::PathBuf;

fn main() {
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
