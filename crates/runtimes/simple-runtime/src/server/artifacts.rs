use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const ARTIFACT_DIR_NAME: &str = "imagetranslator-artifacts";
const SOURCE_DIR: &str = "src";
const PREVIEW_DIR: &str = "preview";

/// 每个 Daemon 实例独占 `<base>/<instance-id>/`，id 以 PID 开头。
/// 启动时只回收 PID 已不存在的实例，所以同机的多个 Daemon 不会清空彼此的文件。
pub struct ArtifactStore {
    root: PathBuf,
}

impl ArtifactStore {
    pub fn platform() -> std::io::Result<Self> {
        Self::open_at(std::env::temp_dir().join(ARTIFACT_DIR_NAME))
    }

    pub fn open_at(base: PathBuf) -> std::io::Result<Self> {
        fs::create_dir_all(&base)?;
        reap_stale(&base, &pid_is_alive, !legacy_daemon_may_run(&base));
        // 同一进程里开两个 store（测试）时 PID 相同，靠随机后缀区分。
        let suffix = &uuid::Uuid::new_v4().simple().to_string()[..8];
        let root = base.join(format!("{}-{suffix}", std::process::id()));
        fs::create_dir_all(root.join(SOURCE_DIR))?;
        fs::create_dir_all(root.join(PREVIEW_DIR))?;
        Ok(Self { root })
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn path_for(&self, job_id: &str, ext: &str) -> PathBuf {
        self.root.join(format!("{job_id}.{ext}"))
    }

    /// 上传落盘目录。multipart 的临时文件直接建在这里，再改名成源文件，不跨文件系统拷贝。
    pub fn upload_dir(&self) -> PathBuf {
        self.root.join(SOURCE_DIR)
    }

    /// Job 的源图。它要活过 done，因为 `GET /jobs/{id}/source` 还要读它。
    pub fn source_path_for(&self, job_id: &str) -> PathBuf {
        self.upload_dir().join(job_id)
    }

    /// 降采样预览的磁盘缓存，键由调用方按 Job id、来源和长边拼出。
    pub fn preview_path_for(&self, key: &str) -> PathBuf {
        self.root.join(PREVIEW_DIR).join(format!("{key}.jpg"))
    }

    /// 退出时只删自己的实例目录。
    pub fn remove_instance(&self) -> std::io::Result<()> {
        match fs::remove_dir_all(&self.root) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

fn instance_pid(dir_name: &str) -> Option<u32> {
    let (pid, suffix) = dir_name.split_once('-')?;
    if suffix.is_empty() || !suffix.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    pid.parse().ok()
}

/// 清理上次运行遗留的内容：PID 已不存在的实例目录，以及旧版直接放在 `base` 下的平铺文件。
/// 旧版 Daemon 没有实例目录，仍在运行时它的文件不能删，所以平铺文件由调用方判断后才清。
fn reap_stale(base: &Path, alive: &dyn Fn(u32) -> bool, legacy_files_orphaned: bool) {
    let Ok(entries) = fs::read_dir(base) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            let name = entry.file_name();
            if let Some(pid) = name.to_str().and_then(instance_pid) {
                if !alive(pid) {
                    let _ = fs::remove_dir_all(&path);
                }
            }
        } else if legacy_files_orphaned {
            let _ = fs::remove_file(&path);
        }
    }
}

#[cfg(unix)]
fn pid_is_alive(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    // 信号 0 只检查进程是否存在。EPERM 说明存在但不属于我们。
    let exists = unsafe { libc::kill(pid, 0) } == 0;
    exists || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(not(unix))]
fn pid_is_alive(_pid: u32) -> bool {
    true
}

/// 旧版 Daemon 不留任何标记，只能看进程表：有同名可执行文件的进程既不是自己，
/// 也没有实例目录，就当它可能是旧版 Daemon。`ps` 失败时同样判为可能在运行，宁可不删。
fn legacy_daemon_may_run(base: &Path) -> bool {
    let Some(exe_name) = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_owned()))
    else {
        return true;
    };
    let Ok(out) = Command::new("ps").args(["-axo", "pid=,comm="]).output() else {
        return true;
    };
    if !out.status.success() {
        return true;
    }
    let claimed: Vec<u32> = fs::read_dir(base)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_str().and_then(instance_pid))
        .collect();
    String::from_utf8_lossy(&out.stdout).lines().any(|line| {
        let Some((pid, comm)) = line.trim_start().split_once(char::is_whitespace) else {
            return false;
        };
        let Ok(pid) = pid.parse::<u32>() else {
            return false;
        };
        pid != std::process::id()
            && !claimed.contains(&pid)
            && Path::new(comm.trim()).file_name() == Some(exe_name.as_os_str())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_base() -> PathBuf {
        std::env::temp_dir().join(format!("it-art-{}", uuid::Uuid::new_v4()))
    }

    #[test]
    fn two_stores_in_one_base_never_touch_each_other() {
        let base = tmp_base();
        let a = ArtifactStore::open_at(base.clone()).unwrap();
        fs::write(a.path_for("job-a", "png"), b"a").unwrap();
        let b = ArtifactStore::open_at(base.clone()).unwrap();
        assert_ne!(a.root(), b.root());
        assert!(
            a.path_for("job-a", "png").exists(),
            "opening B must not wipe A"
        );
        fs::write(b.path_for("job-b", "png"), b"b").unwrap();
        b.remove_instance().unwrap();
        assert!(
            a.path_for("job-a", "png").exists(),
            "B exiting must not wipe A"
        );
        assert!(!b.root().exists());
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn instance_dir_name_starts_with_the_pid() {
        let base = tmp_base();
        let store = ArtifactStore::open_at(base.clone()).unwrap();
        let name = store.root().file_name().unwrap().to_str().unwrap();
        assert_eq!(instance_pid(name), Some(std::process::id()));
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn reap_removes_only_dead_instances() {
        let base = tmp_base();
        let live = base.join("111-aaaaaaaa");
        let dead = base.join("222-bbbbbbbb");
        let foreign = base.join("not-an-instance");
        for d in [&live, &dead, &foreign] {
            fs::create_dir_all(d).unwrap();
            fs::write(d.join("x.png"), b"x").unwrap();
        }
        reap_stale(&base, &|pid| pid == 111, false);
        assert!(live.join("x.png").exists());
        assert!(!dead.exists());
        assert!(
            foreign.join("x.png").exists(),
            "unknown directories are not ours"
        );
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn legacy_flat_files_survive_while_an_old_daemon_may_own_them() {
        let base = tmp_base();
        fs::create_dir_all(&base).unwrap();
        fs::write(base.join("old-job.png"), b"stale").unwrap();
        reap_stale(&base, &|_| true, false);
        assert!(base.join("old-job.png").exists());
        reap_stale(&base, &|_| true, true);
        assert!(!base.join("old-job.png").exists());
        fs::remove_dir_all(base).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn pid_probe_tells_a_reaped_child_from_ourselves() {
        let mut child = Command::new("sh").args(["-c", "exit 0"]).spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        assert!(!pid_is_alive(pid));
        assert!(pid_is_alive(std::process::id()));
    }

    #[cfg(unix)]
    #[test]
    fn stale_dir_of_a_reaped_process_is_removed_on_open() {
        let mut child = Command::new("sh").args(["-c", "exit 0"]).spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        let base = tmp_base();
        let stale = base.join(format!("{pid}-cafe0123"));
        fs::create_dir_all(&stale).unwrap();
        fs::write(stale.join("leftover.png"), b"x").unwrap();
        let store = ArtifactStore::open_at(base.clone()).unwrap();
        assert!(!stale.exists());
        assert!(store.root().exists());
        fs::remove_dir_all(base).unwrap();
    }
}
