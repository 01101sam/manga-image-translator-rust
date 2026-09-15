use std::time::Duration;

use ort::execution_providers::{CUDAExecutionProvider, ExecutionProvider};
use reqwest::Client;
use semver::Version;
use serde::Deserialize;

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
}

pub fn check_cuda() -> bool {
    let cuda = CUDAExecutionProvider::default();
    cuda.is_available().unwrap_or_default()
}

pub async fn check_crate_version(repo: &str) -> Result<bool, Box<dyn std::error::Error>> {
    let url = format!("https://api.github.com/repos/{}/releases/latest", repo);

    let client = Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;
    let release: Release = client
        .get(&url)
        .header("User-Agent", "reqwest")
        .send()
        .await?
        .json()
        .await?;

    let latest_version = Version::parse(release.tag_name.trim_start_matches('v'))?;
    let current_version = Version::parse(env!("CARGO_PKG_VERSION"))?;

    if current_version < latest_version {
        eprintln!(
            "⚠️  Version is outdated (current: {}, latest: {}).\n👉 See: {}",
            current_version, latest_version, release.html_url
        );
        Ok(false)
    } else {
        Ok(true)
    }
}
