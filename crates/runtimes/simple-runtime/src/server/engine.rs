use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use html::HtmlRenderer;
use parking_lot::Mutex;
use png::PngRenderer;
use serde::Serialize;
use tokio::runtime::Handle;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::render_config;
use crate::server::artifacts::ArtifactStore;
use crate::server::jobs::JobStore;
use crate::server::queue::Queue;
use crate::settings::{Renderer, Settings};
use crate::setup::Models;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EngineState {
    Stopped,
    Starting,
    Running,
    Draining,
}

impl EngineState {
    pub fn accepts_jobs(self) -> bool {
        matches!(self, EngineState::Starting | EngineState::Running)
    }
}

pub struct WorkerParams {
    pub batch_ocr: usize,
    pub batch_upscaler: usize,
    pub cuda: bool,
}

struct WorkerCtx {
    queue: Arc<Queue>,
    jobs: Arc<JobStore>,
    artifacts: Arc<ArtifactStore>,
    running: Arc<AtomicUsize>,
    stop: watch::Receiver<bool>,
    params: WorkerParams,
    engine_state: Arc<Mutex<EngineState>>,
    ready: Arc<AtomicUsize>,
    expected: usize,
}

pub struct Engine {
    state: Arc<Mutex<EngineState>>,
    workers: Vec<JoinHandle<()>>,
    stop_tx: Option<watch::Sender<bool>>,
    running: Arc<AtomicUsize>,
    ready: Arc<AtomicUsize>,
    runtime: Handle,
    queue: Arc<Queue>,
    jobs: Arc<JobStore>,
    artifacts: Arc<ArtifactStore>,
}

impl Engine {
    pub fn new(
        queue: Arc<Queue>,
        jobs: Arc<JobStore>,
        artifacts: Arc<ArtifactStore>,
        runtime: Handle,
    ) -> Self {
        Self {
            state: Arc::new(Mutex::new(EngineState::Stopped)),
            workers: Vec::new(),
            stop_tx: None,
            running: Arc::new(AtomicUsize::new(0)),
            ready: Arc::new(AtomicUsize::new(0)),
            runtime,
            queue,
            jobs,
            artifacts,
        }
    }

    pub fn state(&self) -> EngineState {
        *self.state.lock()
    }

    fn set_state(&self, next: EngineState) {
        *self.state.lock() = next;
    }

    #[cfg(test)]
    fn on_worker_ready(&self, expected: usize) {
        on_worker_ready(&self.state, &self.ready, expected);
    }

    pub async fn start(&mut self, n: usize, params: WorkerParams) {
        if matches!(self.state(), EngineState::Starting | EngineState::Running) {
            return;
        }
        self.ready.store(0, Ordering::SeqCst);
        self.set_state(if n == 0 {
            EngineState::Running
        } else {
            EngineState::Starting
        });
        let (tx, rx) = watch::channel(false);
        self.stop_tx = Some(tx);
        let mut workers = Vec::with_capacity(n);
        for _ in 0..n {
            let ctx = WorkerCtx {
                queue: self.queue.clone(),
                jobs: self.jobs.clone(),
                artifacts: self.artifacts.clone(),
                running: self.running.clone(),
                stop: rx.clone(),
                params: WorkerParams {
                    batch_ocr: params.batch_ocr,
                    batch_upscaler: params.batch_upscaler,
                    cuda: params.cuda,
                },
                engine_state: self.state.clone(),
                ready: self.ready.clone(),
                expected: n,
            };
            workers.push(self.runtime.spawn(worker_loop(ctx)));
        }
        self.workers = workers;
    }

    pub async fn stop(&mut self) {
        if self.state() == EngineState::Stopped {
            return;
        }
        self.set_state(EngineState::Draining);
        let queued = self.queue.drain();
        self.jobs.cancel_ids(&queued);
        while self.running.load(Ordering::SeqCst) > 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        if let Some(tx) = self.stop_tx.take() {
            let _ = tx.send(true);
        }
        self.queue.wake_all();
        for worker in self.workers.drain(..) {
            let _ = worker.await;
        }
        self.set_state(EngineState::Stopped);
    }

    pub async fn restart(&mut self, n: usize, params: WorkerParams) {
        self.stop().await;
        self.start(n, params).await;
    }
}

fn on_worker_ready(state: &Mutex<EngineState>, ready: &AtomicUsize, expected: usize) {
    let n = ready.fetch_add(1, Ordering::SeqCst) + 1;
    let mut g = state.lock();
    if *g == EngineState::Starting && n >= expected {
        *g = EngineState::Running;
    }
}

async fn worker_loop(mut ctx: WorkerCtx) {
    let mut models = Models::new(
        ctx.params.batch_upscaler,
        ctx.params.batch_ocr,
        true,
        ctx.params.cuda,
    )
    .await;
    on_worker_ready(&ctx.engine_state, &ctx.ready, ctx.expected);
    loop {
        let Some(id) = ctx.queue.recv(&mut ctx.stop).await else {
            break;
        };
        let Some(work) = ctx.jobs.take_for_run(&id) else {
            continue;
        };
        ctx.running.fetch_add(1, Ordering::SeqCst);
        let result = models
            .execute(work.image, &work.settings, None, None)
            .await;
        match result {
            Ok(Some(exp)) => {
                let ext = work.settings.render.renderer.extension();
                let path = ctx.artifacts.path_for(&id, ext);
                match write_render(exp, &work.settings, &path) {
                    Ok(()) => {
                        let mime = mime_of(&work.settings.render.renderer);
                        let _ = ctx.jobs.finish_ok(&id, path, mime.into());
                    }
                    Err(e) => {
                        let _ = ctx.jobs.finish_err(&id, e);
                    }
                }
            }
            Ok(None) => {
                let _ = ctx.jobs.finish_err(&id, "未检测到可翻译内容".into());
            }
            Err(e) => {
                let _ = ctx.jobs.finish_err(&id, format!("执行失败: {e}"));
            }
        }
        ctx.running.fetch_sub(1, Ordering::SeqCst);
    }
}

pub fn mime_of(renderer: &Renderer) -> &'static str {
    match renderer {
        Renderer::Png => "image/png",
        Renderer::Html => "text/html",
        Renderer::Raw => "application/octet-stream",
    }
}

fn write_render(exp: export::Export, settings: &Settings, output: &Path) -> Result<(), String> {
    match settings.render.renderer {
        Renderer::Html => {
            let (data, _) = HtmlRenderer::render(vec![exp], None, false);
            File::create(output)
                .and_then(|mut f| f.write_all(&data))
                .map_err(|e| format!("写入渲染结果失败: {e}"))?;
        }
        Renderer::Raw => {
            File::create(output)
                .and_then(|mut f| f.write_all(&exp.export()))
                .map_err(|e| format!("写入渲染结果失败: {e}"))?;
        }
        Renderer::Png => {
            let mut png = PngRenderer::default();
            let img = png
                .render(exp, render_config(&settings.render))
                .map_err(|e| format!("渲染失败: {e}"))?;
            img.to_image()
                .map_err(|e| format!("编码 PNG 失败: {e}"))?
                .save(output)
                .map_err(|e| format!("保存 PNG 失败: {e}"))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::artifacts::ArtifactStore;
    use crate::server::jobs::JobStore;
    use crate::server::queue::Queue;

    fn test_engine() -> (Engine, tokio::runtime::Runtime) {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let root = std::env::temp_dir().join(format!("it-eng-{}", uuid::Uuid::new_v4()));
        let engine = Engine::new(
            Arc::new(Queue::new()),
            Arc::new(JobStore::new()),
            Arc::new(ArtifactStore::open_at(root).unwrap()),
            rt.handle().clone(),
        );
        (engine, rt)
    }

    #[test]
    fn starting_stays_until_every_worker_is_ready() {
        let (engine, _rt) = test_engine();
        engine.set_state(EngineState::Starting);
        engine.ready.store(0, Ordering::SeqCst);
        assert_eq!(engine.state(), EngineState::Starting);
        engine.on_worker_ready(2);
        assert_eq!(engine.state(), EngineState::Starting);
        engine.on_worker_ready(2);
        assert_eq!(engine.state(), EngineState::Running);
    }

    #[test]
    fn zero_workers_start_as_running() {
        let (mut engine, rt) = test_engine();
        rt.block_on(async {
            engine
                .start(
                    0,
                    WorkerParams {
                        batch_ocr: 1,
                        batch_upscaler: 1,
                        cuda: false,
                    },
                )
                .await;
            assert_eq!(engine.state(), EngineState::Running);
        });
    }
}
