use std::{
    fs::{self, read_dir, read_to_string, File, OpenOptions},
    io::{BufReader, Read, Seek as _, Write as _},
    path::{Component, Path, PathBuf},
};

use base_util::project::root_path;
use flate2::read::GzDecoder;
use indicatif::{ProgressBar, ProgressStyle};
use log::{debug, info};
use sha2::{Digest, Sha256};
use tar::Archive;

pub struct ModelDb {}

impl ModelDb {
    pub fn get(
        &self,
        kind: &str,
        name: &str,
        file: &str,
        url: &str,
        hash: &str,
    ) -> anyhow::Result<PathBuf> {
        let base_path = root_path().join("models").join(kind).join(name);
        let mut file_path = base_path.join(file);

        std::fs::create_dir_all(file_path.parent().expect("set above"))?;
        let mut folder = false;
        // allow:clone[pathbuf]
        let ret_file_path = file_path.clone();
        if file.contains("/") {
            file_path = file_path.parent().expect("set above").to_path_buf();

            folder = true;
        }
        if failure(Some(&base_path), &file_path, hash) {
            download_and_extract(url, &file_path, folder)?;
            if failure(Some(&base_path), &file_path, hash) {
                let _ = std::fs::remove_file(&file_path);
                download_and_extract(url, &file_path, folder)?;
                if failure(Some(&base_path), &file_path, hash) {
                    let _ = std::fs::remove_file(&file_path);
                    download_and_extract(url, &file_path, folder)?;
                    if failure(Some(&base_path), &file_path, hash) {
                        panic!()
                    }
                }
            }
        }
        Ok(ret_file_path)
    }
}

fn get_all_files_recursively<P: AsRef<Path>>(dir: P) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();

    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                files.extend(get_all_files_recursively(&path));
            } else if path.is_file() {
                files.push(path);
            }
        }
    }

    files
}

fn failure<P: AsRef<Path>>(base_path: Option<P>, file_path: P, expected_hash: &str) -> bool {
    if !file_path.as_ref().exists() {
        return true;
    }
    if file_path.as_ref().is_dir() {
        let files = read_dir(file_path.as_ref())
            .unwrap()
            .filter_map(|v| v.ok())
            .filter(|v| !v.file_name().to_str().unwrap_or(".").starts_with("."))
            .count();
        if files == 0 {
            return true;
        }
    }
    if expected_hash == "###" {
        return false;
    }

    if let Some(base_path) = &base_path {
        let base_path = base_path.as_ref();
        let info_path = base_path.join("hashes");
        let p = file_path
            .as_ref()
            .strip_prefix(base_path)
            .unwrap_or(file_path.as_ref());
        let content = read_to_string(&info_path).unwrap_or_default();
        let hash_cache = content
            .lines()
            .filter_map(|v| v.trim().rsplit_once(" "))
            .find(|v| Path::new(v.0) == p)
            .map(|v| v.1);
        if let Some(hash) = hash_cache {
            if hash != expected_hash {
                let content = content
                    .lines()
                    .filter_map(|v| v.trim().rsplit_once(" "))
                    .filter(|v| Path::new(v.0) != p)
                    .map(|v| format!("{} {}", v.0, v.1))
                    .collect::<Vec<_>>()
                    .join("\n");
                let mut file = OpenOptions::new()
                    .write(true)
                    .truncate(true)
                    .open(&info_path)
                    .unwrap();

                file.write_all(content.as_bytes()).unwrap();
            }
            return hash != expected_hash;
        }
    }

    match file_path.as_ref().is_dir() {
        true => {
            let mut entries = Vec::new();

            if let Ok(walk) = fs::read_dir(file_path.as_ref()) {
                for entry in walk.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        entries.extend(get_all_files_recursively(&path));
                    } else if path.is_file() {
                        entries.push(path);
                    }
                }
            }

            entries.sort();

            let mut hasher = Sha256::new();

            for path in entries {
                if let Ok(rel_path) = path.strip_prefix(file_path.as_ref()) {
                    hasher.update(rel_path.to_string_lossy().as_bytes());
                }

                let mut file = match File::open(&path) {
                    Ok(f) => f,
                    Err(_) => return true,
                };

                let mut buffer = Vec::new();
                if file.read_to_end(&mut buffer).is_err() {
                    return true;
                }
                hasher.update(&buffer);
            }

            let result = hasher.finalize();
            let dir_hash = format!("{:x}", result);
            debug!("Dir hash: {}", dir_hash);
            if let Some(base_path) = base_path {
                if &dir_hash == expected_hash {
                    let mut file = OpenOptions::new()
                        .append(true)
                        .create(true)
                        .open(base_path.as_ref().join("hashes"))
                        .unwrap();

                    writeln!(
                        file,
                        "{} {}",
                        file_path
                            .as_ref()
                            .strip_prefix(base_path.as_ref())
                            .unwrap_or(file_path.as_ref())
                            .to_string_lossy(),
                        expected_hash
                    )
                    .unwrap();
                }
            }
            dir_hash != expected_hash.to_owned()
        }
        false => {
            let mut file = match std::fs::File::open(&file_path) {
                Ok(f) => f,
                Err(_) => return true,
            };

            let mut hasher = Sha256::new();
            let mut buffer = Vec::new();
            if file.read_to_end(&mut buffer).is_err() {
                return true;
            }

            hasher.update(&buffer);
            let result = hasher.finalize();
            let file_hash = format!("{:x}", result);
            debug!("File hash: {}", file_hash);
            if let Some(base_path) = base_path {
                if &file_hash == expected_hash {
                    let mut file = OpenOptions::new()
                        .append(true)
                        .create(true)
                        .open(base_path.as_ref().join("hashes"))
                        .unwrap();

                    writeln!(
                        file,
                        "{} {}",
                        file_path
                            .as_ref()
                            .strip_prefix(base_path.as_ref())
                            .unwrap_or(file_path.as_ref())
                            .to_string_lossy(),
                        expected_hash
                    )
                    .unwrap();
                }
            }

            &file_hash != expected_hash
        }
    }
}

fn download_and_extract(url: &str, file_path: &Path, folder: bool) -> anyhow::Result<()> {
    info!("Downloading from: {}", url);

    let mut response = ureq::get(url).call()?;
    let total_size = response
        .headers()
        .get("Content-Length")
        .and_then(|val| val.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);

    let pb = if total_size > 0 {
        let pb = ProgressBar::new(total_size);
        if let Ok(style) = ProgressStyle::default_bar()
            .template("{msg} [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})")
        {
            pb.set_style(style.progress_chars("#>-"));
        }
        pb.set_message("Downloading");
        Some(pb)
    } else {
        None
    };

    struct ProgressReader<R> {
        inner: R,
        progress_bar: Option<ProgressBar>,
        bytes_read: u64,
    }

    impl<R: Read> Read for ProgressReader<R> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = self.inner.read(buf)?;
            self.bytes_read += n as u64;
            if let Some(pb) = &self.progress_bar {
                pb.set_position(self.bytes_read);
            }
            Ok(n)
        }
    }

    let mut temp_file = tempfile::tempfile()?;

    let b = response.body_mut();
    let b = b.as_reader();
    let mut progress_reader = ProgressReader {
        inner: b,
        // allow:clone[arc]
        progress_bar: pb.clone(),
        bytes_read: 0,
    };

    std::io::copy(&mut progress_reader, &mut temp_file)?;

    if let Some(pb) = pb {
        pb.finish_with_message("Download complete");
    }
    let url = url.split_once("?").map(|v| v.0).unwrap_or(url);
    if url.ends_with(".tar.gz") {
        debug!("Extracting archive...");

        temp_file.rewind()?;

        let buf_reader = BufReader::new(temp_file);
        let decoder = GzDecoder::new(buf_reader);
        let archive = Archive::new(decoder);

        let extract_dir = if folder {
            std::fs::create_dir_all(file_path)?;
            file_path
        } else {
            file_path.parent().expect("file_path must have parent")
        };

        unpack_without_top_dir(archive, extract_dir)?;
        debug!("Extraction complete.");
    } else {
        debug!("Downloaded file is not a .tar.gz archive, saving as normal file.");
        temp_file.rewind()?;
        let mut output = File::create(file_path)?;
        std::io::copy(&mut temp_file, &mut output)?;
    }

    Ok(())
}

fn normalize_join(target_dir: &Path, relative_path: &Path) -> PathBuf {
    let cleaned = relative_path.strip_prefix(".").unwrap_or(relative_path);

    let mut rel_components = cleaned.components();
    let tar_comp = target_dir.components();
    let first = tar_comp.last();
    let second = rel_components.next();

    if let (Some(Component::Normal(first)), Some(Component::Normal(second))) = (first, second) {
        if first == second {
            return target_dir.join(rel_components.collect::<PathBuf>());
        }
    }

    target_dir.join(cleaned)
}
fn unpack_without_top_dir<R: std::io::Read>(
    mut archive: Archive<R>,
    target_dir: &Path,
) -> std::io::Result<()> {
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?;
        let components = path.components();

        let relative_path: PathBuf = components.collect();
        let out_path = normalize_join(target_dir, &relative_path);
        if let Some(name) = out_path.file_name().and_then(|v| v.to_str()) {
            if name.starts_with(".") {
                continue;
            }
        }

        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        entry.unpack(out_path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;
    //TODO: test successfull download

    #[test]
    fn hashing() {
        let _ = env_logger::builder()
            .is_test(true)
            .filter_level(log::LevelFilter::Debug)
            .try_init();
        assert_eq!(
            failure(
                None,
                root_path().join("models/detector/dbnet/model.onnx"),
                ""
            ),
            true
        );
    }

    #[test]
    fn test_failure_returns_true_for_nonexistent_file() {
        let path = PathBuf::from("nonexistent.file");
        assert!(failure(None, path, "abc"));
    }

    #[test]
    fn test_failure_returns_false_for_correct_hash() {
        let dir = tempdir().expect("couldnt create tempdir");
        let path = dir.path().join("test.txt");
        fs::write(&path, "correct content").expect("couldnt write temp file");

        let correct_hash = format!("{:x}", Sha256::digest(b"correct content"));
        assert!(!failure(None, &path, &correct_hash));
    }

    #[test]
    #[should_panic]
    fn test_get_panics_on_double_hash_failure() {
        let db = ModelDb {};
        let _ = db.get(
            "invalid",
            "invalid",
            "bad.txt",
            "https://example.com/404.tar.gz",
            "invalidhash",
        );
    }

    #[test]
    fn hashing2() {
        let _ = env_logger::builder()
            .is_test(true)
            .filter_level(log::LevelFilter::Debug)
            .try_init();
        assert_eq!(
            failure(
                None,
                root_path().join("models/invalid/invalid/spm.nopretok"),
                ""
            ),
            true
        );
    }

    // Optional: test download_and_extract with a fake URL
    // (would need a test server or mock ureq)
}
