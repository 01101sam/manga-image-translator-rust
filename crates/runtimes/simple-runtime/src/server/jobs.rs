use std::collections::HashMap;

use image::DynamicImage;
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

pub struct JobWork {
    pub image: DynamicImage,
    pub settings: Settings,
}

struct Job {
    state: JobState,
    error: Option<String>,
    artifact: Option<std::path::PathBuf>,
    mime: Option<String>,
    renderer: Renderer,
    image: Option<DynamicImage>,
    settings: Settings,
}

pub struct JobView {
    pub state: JobState,
    pub error: Option<String>,
    pub artifact: Option<std::path::PathBuf>,
    pub mime: Option<String>,
    pub renderer: Renderer,
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

    pub fn create(&self, image: DynamicImage, settings: Settings) -> String {
        let id = Uuid::new_v4().to_string();
        let renderer = match settings.render.renderer {
            Renderer::Png => Renderer::Png,
            Renderer::Html => Renderer::Html,
            Renderer::Raw => Renderer::Raw,
        };
        self.jobs.lock().insert(
            id.clone(),
            Job {
                state: JobState::Queued,
                error: None,
                artifact: None,
                mime: None,
                renderer,
                image: Some(image),
                settings,
            },
        );
        id
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

    pub fn discard(&self, id: &str) {
        self.jobs.lock().remove(id);
    }

    pub fn cancel(&self, id: &str) -> Result<JobState, CancelError> {
        let mut g = self.jobs.lock();
        let job = g.get_mut(id).ok_or(CancelError::NotFound)?;
        match job.state.cancel() {
            Ok(next) => {
                job.state = next;
                job.image = None;
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
                    job.image = None;
                }
            }
        }
    }

    pub fn take_for_run(&self, id: &str) -> Option<JobWork> {
        let mut g = self.jobs.lock();
        let job = g.get_mut(id)?;
        job.state = job.state.start().ok()?;
        let image = job.image.take()?;
        Some(JobWork {
            image,
            settings: clone_settings(&job.settings),
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

    fn dummy_image() -> DynamicImage {
        DynamicImage::new_rgb8(1, 1)
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
        let id = store.create(dummy_image(), Settings::default());
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
        let id = store.create(dummy_image(), Settings::default());
        assert_eq!(store.cancel(&id).unwrap(), JobState::Cancelled);
        assert_eq!(store.get(&id).unwrap().state, JobState::Cancelled);
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
