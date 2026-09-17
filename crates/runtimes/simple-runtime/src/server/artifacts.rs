use std::fs;
use std::path::{Path, PathBuf};

pub const ARTIFACT_DIR_NAME: &str = "imagetranslator-artifacts";

pub struct ArtifactStore {
    root: PathBuf,
}

impl ArtifactStore {
    pub fn platform() -> std::io::Result<Self> {
        Self::open_at(std::env::temp_dir().join(ARTIFACT_DIR_NAME))
    }

    pub fn open_at(root: PathBuf) -> std::io::Result<Self> {
        let store = Self { root };
        store.wipe()?;
        Ok(store)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn path_for(&self, job_id: &str, ext: &str) -> PathBuf {
        self.root.join(format!("{job_id}.{ext}"))
    }

    pub fn wipe(&self) -> std::io::Result<()> {
        if self.root.exists() {
            fs::remove_dir_all(&self.root)?;
        }
        fs::create_dir_all(&self.root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_root() -> PathBuf {
        std::env::temp_dir().join(format!("it-art-{}", uuid::Uuid::new_v4()))
    }

    #[test]
    fn startup_wipe_removes_leftovers() {
        let root = tmp_root();
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("old.bin"), b"stale").unwrap();
        let store = ArtifactStore::open_at(root.clone()).unwrap();
        assert!(!store.root().join("old.bin").exists());
        assert!(store.root().exists());
    }

    #[test]
    fn shutdown_wipe_empties_dir() {
        let root = tmp_root();
        let store = ArtifactStore::open_at(root.clone()).unwrap();
        fs::write(store.path_for("job1", "png"), b"img").unwrap();
        assert!(store.path_for("job1", "png").exists());
        store.wipe().unwrap();
        assert!(store.root().read_dir().unwrap().next().is_none());
    }
}
