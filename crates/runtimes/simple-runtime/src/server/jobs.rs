use std::collections::HashMap;
use std::path::PathBuf;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::settings::{Renderer, Settings};

const ALLOWED_OVERRIDES: &[&str] = &["target_lang", "detector", "translator"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobState {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionError {
    pub from: JobState,
    pub to: JobState,
}

impl JobState {
    pub fn start(self) -> Result<Self, TransitionError> {
        match self {
            JobState::Queued => Ok(JobState::Running),
            from => Err(TransitionError {
                from,
                to: JobState::Running,
            }),
        }
    }

    pub fn finish(self) -> Result<Self, TransitionError> {
        match self {
            JobState::Running => Ok(JobState::Done),
            from => Err(TransitionError {
                from,
                to: JobState::Done,
            }),
        }
    }

    pub fn fail(self) -> Result<Self, TransitionError> {
        match self {
            JobState::Running => Ok(JobState::Failed),
            from => Err(TransitionError {
                from,
                to: JobState::Failed,
            }),
        }
    }

    pub fn cancel(self) -> Result<Self, TransitionError> {
        match self {
            JobState::Queued => Ok(JobState::Cancelled),
            from => Err(TransitionError {
                from,
                to: JobState::Cancelled,
            }),
        }
    }
}

#[derive(Debug)]
pub enum OverrideError {
    InvalidJson,
    RejectedKey(String),
    InvalidValue(String),
}

impl OverrideError {
    pub fn message(&self) -> String {
        match self {
            OverrideError::InvalidJson => "overrides JSON is invalid".into(),
            OverrideError::RejectedKey(k) => format!("rejected override key: {k}"),
            OverrideError::InvalidValue(k) => format!("invalid override value for {k}"),
        }
    }
}

pub fn apply_overrides(base: &Settings, raw: &str) -> Result<Settings, OverrideError> {
    let value: Value = if raw.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(raw).map_err(|_| OverrideError::InvalidJson)?
    };
    let obj = value.as_object().ok_or(OverrideError::InvalidJson)?;
    for key in obj.keys() {
        if !ALLOWED_OVERRIDES.contains(&key.as_str()) {
            return Err(OverrideError::RejectedKey(key.clone()));
        }
    }
    let mut merged = serde_json::to_value(base).map_err(|_| OverrideError::InvalidJson)?;
    if let Some(lang) = obj.get("target_lang") {
        merged["translator"]["target"] = lang.clone();
    }
    if let Some(det) = obj.get("detector") {
        merged["detector"]["detector"] = det.clone();
    }
    if let Some(translator) = obj.get("translator") {
        match translator {
            Value::String(_) => {
                merged["translator"]["mode"] = translator.clone();
            }
            Value::Object(src) => {
                if let Some(Value::Object(dst)) = merged.get_mut("translator") {
                    for (k, v) in src {
                        dst.insert(k.clone(), v.clone());
                    }
                }
            }
            _ => return Err(OverrideError::InvalidValue("translator".into())),
        }
    }
    serde_json::from_value(merged).map_err(|_| OverrideError::InvalidJson)
}

pub fn clone_settings(settings: &Settings) -> Settings {
    serde_json::from_value(serde_json::to_value(settings).expect("settings serialize"))
        .expect("settings deserialize")
}

/// 排队的 Job 只带源文件路径，像素在 Worker 取走时才解码，队列长度不会放大内存。
pub struct JobSpec {
    pub source: PathBuf,
    pub source_mime: &'static str,
    pub settings: Settings,
    pub downscale_long_edge: Option<u32>,
}

pub struct JobWork {
    pub source: PathBuf,
    pub settings: Settings,
    pub downscale_long_edge: Option<u32>,
}

struct Job {
    state: JobState,
    error: Option<String>,
    artifact: Option<PathBuf>,
    mime: Option<String>,
    renderer: Renderer,
    spec: JobSpec,
}

pub struct JobView {
    pub state: JobState,
    pub error: Option<String>,
    pub artifact: Option<PathBuf>,
    pub mime: Option<String>,
    pub renderer: Renderer,
    pub source: PathBuf,
    pub source_mime: &'static str,
}

pub struct JobListItem {
    pub job_id: String,
    pub state: JobState,
}

pub struct JobStore {
    jobs: Mutex<HashMap<String, Job>>,
}

impl JobStore {
    pub fn new() -> Self {
        Self {
            jobs: Mutex::new(HashMap::new()),
        }
    }

    /// 源文件路径依赖 Job id，所以 id 要先于 `create` 生成。
    pub fn new_id() -> String {
        Uuid::new_v4().to_string()
    }

    pub fn create(&self, id: String, spec: JobSpec) {
        let renderer = match spec.settings.render.renderer {
            Renderer::Png => Renderer::Png,
            Renderer::Html => Renderer::Html,
            Renderer::Raw => Renderer::Raw,
        };
        self.jobs.lock().insert(
            id,
            Job {
                state: JobState::Queued,
                error: None,
                artifact: None,
                mime: None,
                renderer,
                spec,
            },
        );
    }

    pub fn get(&self, id: &str) -> Option<JobView> {
        self.jobs.lock().get(id).map(|j| JobView {
            state: j.state,
            error: j.error.clone(),
            artifact: j.artifact.clone(),
            mime: j.mime.clone(),
            renderer: match j.renderer {
                Renderer::Png => Renderer::Png,
                Renderer::Html => Renderer::Html,
                Renderer::Raw => Renderer::Raw,
            },
            source: j.spec.source.clone(),
            source_mime: j.spec.source_mime,
        })
    }

    pub fn list(&self) -> Vec<JobListItem> {
        self.jobs
            .lock()
            .iter()
            .map(|(id, j)| JobListItem {
                job_id: id.clone(),
                state: j.state,
            })
            .collect()
    }

    /// 入队失败时撤销 Job，同时删掉已落盘的源文件。
    pub fn discard(&self, id: &str) {
        if let Some(job) = self.jobs.lock().remove(id) {
            let _ = std::fs::remove_file(&job.spec.source);
        }
    }

    pub fn cancel(&self, id: &str) -> Result<JobState, CancelError> {
        let mut g = self.jobs.lock();
        let job = g.get_mut(id).ok_or(CancelError::NotFound)?;
        match job.state.cancel() {
            Ok(next) => {
                job.state = next;
                let _ = std::fs::remove_file(&job.spec.source);
                Ok(next)
            }
            Err(e) => Err(CancelError::NotQueued { state: e.from }),
        }
    }

    pub fn cancel_ids(&self, ids: &[String]) {
        let mut g = self.jobs.lock();
        for id in ids {
            if let Some(job) = g.get_mut(id) {
                if let Ok(next) = job.state.cancel() {
                    job.state = next;
                    let _ = std::fs::remove_file(&job.spec.source);
                }
            }
        }
    }

    pub fn take_for_run(&self, id: &str) -> Option<JobWork> {
        let mut g = self.jobs.lock();
        let job = g.get_mut(id)?;
        job.state = job.state.start().ok()?;
        Some(JobWork {
            source: job.spec.source.clone(),
            settings: clone_settings(&job.spec.settings),
            downscale_long_edge: job.spec.downscale_long_edge,
        })
    }

    pub fn finish_ok(
        &self,
        id: &str,
        artifact: std::path::PathBuf,
        mime: String,
    ) -> Result<(), TransitionError> {
        let mut g = self.jobs.lock();
        let job = g.get_mut(id).ok_or(TransitionError {
            from: JobState::Failed,
            to: JobState::Done,
        })?;
        job.state = job.state.finish()?;
        job.artifact = Some(artifact);
        job.mime = Some(mime);
        Ok(())
    }

    pub fn finish_err(&self, id: &str, error: String) -> Result<(), TransitionError> {
        let mut g = self.jobs.lock();
        let job = g.get_mut(id).ok_or(TransitionError {
            from: JobState::Failed,
            to: JobState::Failed,
        })?;
        job.state = job.state.fail()?;
        job.error = Some(error);
        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum CancelError {
    NotFound,
    NotQueued { state: JobState },
}

impl Default for JobStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec_at(source: PathBuf) -> JobSpec {
        JobSpec {
            source,
            source_mime: "image/png",
            settings: Settings::default(),
            downscale_long_edge: Some(4096),
        }
    }

    fn create_job(store: &JobStore) -> String {
        let id = JobStore::new_id();
        store.create(id.clone(), spec_at(PathBuf::from("/nonexistent/source")));
        id
    }

    fn create_job_with_file(store: &JobStore) -> (String, PathBuf) {
        let id = JobStore::new_id();
        let path = std::env::temp_dir().join(format!("it-job-{id}"));
        std::fs::write(&path, b"bytes").unwrap();
        store.create(id.clone(), spec_at(path.clone()));
        (id, path)
    }

    #[test]
    fn queued_to_running_to_done() {
        assert_eq!(JobState::Queued.start().unwrap(), JobState::Running);
        assert_eq!(JobState::Running.finish().unwrap(), JobState::Done);
    }

    #[test]
    fn queued_to_running_to_failed() {
        assert_eq!(JobState::Running.fail().unwrap(), JobState::Failed);
    }

    #[test]
    fn queued_to_cancelled() {
        assert_eq!(JobState::Queued.cancel().unwrap(), JobState::Cancelled);
    }

    #[test]
    fn cancel_on_running_is_error() {
        let err = JobState::Running.cancel().unwrap_err();
        assert_eq!(err.from, JobState::Running);
        assert_eq!(err.to, JobState::Cancelled);
    }

    #[test]
    fn cancel_on_done_failed_cancelled_is_error() {
        assert!(JobState::Done.cancel().is_err());
        assert!(JobState::Failed.cancel().is_err());
        assert!(JobState::Cancelled.cancel().is_err());
    }

    #[test]
    fn illegal_finish_from_queued() {
        assert!(JobState::Queued.finish().is_err());
        assert!(JobState::Queued.fail().is_err());
        assert!(JobState::Done.start().is_err());
    }

    #[test]
    fn store_cancel_running_returns_not_queued() {
        let store = JobStore::new();
        let id = create_job(&store);
        assert!(store.take_for_run(&id).is_some());
        let err = store.cancel(&id).unwrap_err();
        assert_eq!(
            err,
            CancelError::NotQueued {
                state: JobState::Running
            }
        );
    }

    #[test]
    fn store_cancel_queued_ok() {
        let store = JobStore::new();
        let id = create_job(&store);
        assert_eq!(store.cancel(&id).unwrap(), JobState::Cancelled);
        assert_eq!(store.get(&id).unwrap().state, JobState::Cancelled);
    }

    #[test]
    fn take_for_run_hands_out_the_path_and_the_downscale_setting() {
        let store = JobStore::new();
        let (id, path) = create_job_with_file(&store);
        let work = store.take_for_run(&id).unwrap();
        assert_eq!(work.source, path);
        assert_eq!(work.downscale_long_edge, Some(4096));
        assert!(
            path.exists(),
            "the source must outlive the run for GET /source"
        );
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn cancel_and_discard_delete_the_source_file() {
        let store = JobStore::new();
        let (cancelled, cancelled_path) = create_job_with_file(&store);
        let (discarded, discarded_path) = create_job_with_file(&store);
        store.cancel(&cancelled).unwrap();
        store.discard(&discarded);
        assert!(!cancelled_path.exists());
        assert!(!discarded_path.exists());
        assert!(store.get(&discarded).is_none());
    }

    #[test]
    fn reject_unknown_override_key() {
        let err = match apply_overrides(&Settings::default(), r#"{"foo":1}"#) {
            Err(e) => e,
            Ok(_) => panic!("expected RejectedKey"),
        };
        match err {
            OverrideError::RejectedKey(k) => assert_eq!(k, "foo"),
            other => panic!("expected RejectedKey, got {other:?}"),
        }
    }

    #[test]
    fn merge_target_lang_over_base() {
        let settings = apply_overrides(&Settings::default(), r#"{"target_lang":"en"}"#).unwrap();
        assert_eq!(settings.translator.target.0.code(), "en");
    }
}
