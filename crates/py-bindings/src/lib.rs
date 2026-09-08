use std::sync::{Arc, OnceLock};

use base_util::onnx::{all_providers, Providers};
use ctd::CtdDetector;
use dbnet::DbNetDetector;

use interface_detector::textlines::Quadrilateral;
use interface_detector::{DefaultOptions, Detector, PreprocessorOptions};
use interface_image::{CpuImageProcessor, ImageOp, RawImage};
use interface_ocr::QuadrilateralInfo;
use interface_translator::{
    AsyncTranslator, Backend, LangIdDetector, Language, LlmConfig, LlmTranslator, ThinkingStrength,
};
use numpy::{
    ndarray::{Array2, Array3},
    IntoPyArray as _, PyArray2, PyArray3, PyArrayMethods, PyReadonlyArray3,
};
use parking_lot::Mutex;
use pyo3::{exceptions::PyRuntimeError, prelude::*};
use tokio::runtime::{Builder, Runtime};

static TOKIO_RT: OnceLock<Runtime> = OnceLock::new();

fn get_runtime() -> &'static Runtime {
    TOKIO_RT.get_or_init(|| {
        Builder::new_multi_thread()
            .worker_threads(8)
            .enable_all()
            .build()
            .expect("Failed to create Tokio runtime")
    })
}

#[pyclass]
pub struct Session {
    processor: Arc<Arc<dyn ImageOp + Send + Sync>>,
    inner: Arc<Vec<Providers>>,
}

#[pyfunction]
pub fn textline_merge_dispatch(
    items: Vec<(
        String,
        Vec<(i64, i64)>,
        f64,
        Option<[u8; 3]>,
        Option<[u8; 3]>,
        f64,
    )>,
    width: u16,
    height: u16,
) -> PyResult<Vec<(String, Vec<Vec<(i64, i64)>>, f64)>> {
    let det = LangIdDetector::new().map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
    let items = items
        .into_iter()
        .map(|v| QuadrilateralInfo {
            text: v.0,
            fg: v.3,
            bg: v.4,
            prob: v.5,
            pos: Arc::new(parking_lot::Mutex::new(Quadrilateral::new(v.1, v.2))),
        })
        .collect::<Vec<_>>();
    let out = textline_merge::dispatch(items.iter().collect(), width, height, &det)
        .map_err(|v| PyRuntimeError::new_err(v.to_string()))?;
    Ok(out
        .into_iter()
        .map(|v| {
            (
                v.text,
                v.lines
                    .into_iter()
                    .map(|v| v.into_iter().map(|v| (v.x, v.y)).collect::<Vec<_>>())
                    .collect::<Vec<_>>(),
                v.angle,
            )
        })
        .collect::<Vec<_>>())
}

#[pymethods]
impl Session {
    #[new]
    /// allowed providers are cuda, coreml, directml, tensorrt
    /// all are enabled by default
    fn new(providers: Option<Vec<String>>) -> Self {
        let providers = match providers {
            None => all_providers(),
            Some(providers) => providers
                .iter()
                .map(|v| match v.as_str() {
                    "cuda" => Providers::CUDA,
                    "coreml" => Providers::CoreML,
                    "directml" => Providers::DirectML,
                    "tensorrt" => Providers::TensorRT,
                    _ => panic!("Invalid provider"),
                })
                .collect(),
        };
        Session {
            inner: Arc::new(providers),
            processor: Arc::new(Arc::new(CpuImageProcessor::default())),
        }
    }

    fn llm_translator(
        &self,
        backend: &str,
        api_key: String,
        model: Option<String>,
        base_url: Option<String>,
        thinking: Option<bool>,
        thinking_strength: Option<String>,
        web_search_key: Option<String>,
    ) -> PyResult<PyTranslator> {
        let backend = match backend {
            "openai" | "open_ai" => Backend::OpenAi,
            "anthropic" => Backend::Anthropic,
            _ => {
                return Err(PyRuntimeError::new_err(
                    "backend must be openai or anthropic",
                ))
            }
        };
        let thinking_strength = match thinking_strength.as_deref() {
            None => ThinkingStrength::High,
            Some(s) => ThinkingStrength::parse(s).ok_or_else(|| {
                PyRuntimeError::new_err("thinking_strength must be low, high, or max")
            })?,
        };
        let base_url = base_url.unwrap_or_else(|| match backend {
            Backend::OpenAi => "https://api.deepseek.com".into(),
            Backend::Anthropic => "https://api.deepseek.com/anthropic".into(),
        });
        Ok(PyTranslator {
            inner: Arc::new(Box::new(LlmTranslator::from_config(LlmConfig {
                backend,
                base_url,
                api_key,
                model: model.unwrap_or_else(|| "deepseek-v4-flash".into()),
                thinking: thinking.unwrap_or(true),
                thinking_strength,
                web_search_key,
                max_iters: 24,
            })) as Box<dyn AsyncTranslator + Send + Sync>),
        })
    }

    fn ctd_detector(&self) -> PyDetector {
        PyDetector {
            inner: Arc::new(Mutex::new(
                // allow:clone[arc]
                Box::new(CtdDetector::new(self.inner.clone())) as Box<dyn Detector + Send + Sync>,
            )),
            // allow:clone[arc]
            processor: self.processor.clone(),
        }
    }

    fn default_detector(&self) -> PyDetector {
        PyDetector {
            inner: Arc::new(Mutex::new(
                // allow:clone[arc]
                Box::new(DbNetDetector::new(self.inner.clone(), false))
                    as Box<dyn Detector + Send + Sync>,
            )),
            // allow:clone[arc]
            processor: self.processor.clone(),
        }
    }

    fn convnext_detector(&self) -> PyDetector {
        PyDetector {
            inner: Arc::new(Mutex::new(
                // allow:clone[arc]
                Box::new(DbNetDetector::new(self.inner.clone(), true))
                    as Box<dyn Detector + Send + Sync>,
            )),
            // allow:clone[arc]
            processor: self.processor.clone(),
        }
    }
}

#[pyclass]
pub struct PyDefaultOptions {
    inner: DefaultOptions,
}

#[pymethods]
impl PyDefaultOptions {
    #[new]
    fn new(detect_size: u64, unclip_ratio: f64, text_threshold: f64, box_threshold: f64) -> Self {
        PyDefaultOptions {
            inner: DefaultOptions {
                detect_size,
                unclip_ratio,
                text_threshold,
                box_threshold,
            },
        }
    }
}

#[pyclass]
pub struct PyPreprocessorOptions {
    inner: PreprocessorOptions,
}

#[pymethods]
impl PyPreprocessorOptions {
    #[new]
    fn new(invert: bool, gamma_correct: bool, rotate: bool, auto_rotate: bool) -> Self {
        PyPreprocessorOptions {
            inner: PreprocessorOptions {
                invert,
                gamma_correct,
                rotate,
                auto_rotate,
            },
        }
    }
}

#[pyclass]
pub struct PyDetector {
    processor: Arc<Arc<dyn ImageOp + Send + Sync>>,
    inner: Arc<Mutex<Box<dyn Detector + Send + Sync>>>,
}

#[pyclass]
pub struct PyTranslator {
    inner: Arc<Box<dyn AsyncTranslator + Send + Sync>>,
}
#[pymethods]
impl PyTranslator {
    pub fn translate<'py>(
        &self,
        py: Python<'py>,
        ocr_json: String,
        tags: Option<String>,
        to: &str,
    ) -> PyResult<String> {
        let to =
            Language::from_name(to).ok_or(PyRuntimeError::new_err("language not supported"))?;
        py.allow_threads(|| {
            get_runtime()
                .block_on(self.inner.translate(&ocr_json, tags.as_deref(), to))
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))
        })
    }
}

#[pyclass]
pub struct PyImage {
    inner: Arc<RawImage>,
}

#[pymethods]
impl PyImage {
    #[new]
    fn new(path: &str) -> PyResult<Self> {
        let v = RawImage::new(path).map_err(|v| PyRuntimeError::new_err(v.to_string()))?;

        Ok(PyImage { inner: Arc::new(v) })
    }

    pub fn to_numpy<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyArray3<u8>>> {
        // allow:clone[python]
        let data = self.inner.data.clone();
        let (width, height, channels) = (
            self.inner.width as usize,
            self.inner.height as usize,
            self.inner.channels as usize,
        );
        let array = Array3::from_shape_vec((height, width, channels), data)
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
        Ok(array.into_pyarray(py))
    }

    #[staticmethod]
    pub fn from_numpy(array: PyReadonlyArray3<u8>) -> PyResult<PyImage> {
        let dims = array.dims();
        let (height, width, channels) = (dims[0], dims[1], dims[2]);

        let array_view = array.as_array().into_iter().map(|v| *v).collect::<Vec<_>>();
        Ok(PyImage {
            inner: Arc::new(RawImage {
                data: array_view,
                width: width as u16,
                height: height as u16,
                channels: channels as u8,
            }),
        })
    }
}

#[pyclass]
pub struct PyQuadrilateral {
    inner: Quadrilateral,
}

#[pymethods]
impl PyQuadrilateral {
    fn score(&self) -> f64 {
        self.inner.score()
    }

    fn aspect_ratio(&self) -> f64 {
        self.inner.aspect_ratio()
    }

    fn area(&self) -> f64 {
        self.inner.area()
    }

    fn vertical(&self) -> bool {
        self.inner.vertical()
    }

    fn pts(&self) -> Vec<(i64, i64)> {
        self.inner
            .pts()
            .iter()
            .map(|v| (v.x, v.y))
            .collect::<Vec<_>>()
    }

    fn structure(&self) -> Vec<(i64, i64)> {
        self.inner
            .structure()
            .iter()
            .map(|v| (v.x, v.y))
            .collect::<Vec<_>>()
    }
}

#[pymethods]
impl PyDetector {
    fn load(&self) -> PyResult<()> {
        get_runtime()
            .block_on(self.inner.lock().reload_())
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))
    }

    fn detect<'py>(
        &mut self,
        py: Python<'py>,
        image: PyRef<PyImage>,
        preprocessor_options: PyRef<PyPreprocessorOptions>,
        options: PyRef<PyDefaultOptions>,
    ) -> PyResult<(Vec<PyQuadrilateral>, Bound<'py, PyArray2<u8>>)> {
        // allow:clone[arc]
        let inner = self.inner.clone();
        let preprocessor_options = preprocessor_options.inner;
        let options = options.inner;
        // allow:clone[arc]
        let img = image.inner.clone();
        // allow:clone[arc]
        let processor = self.processor.clone();
        let det = py
            .allow_threads(|| {
                get_runtime().block_on(inner.lock().detect(
                    &img,
                    preprocessor_options,
                    options,
                    &*processor,
                ))
            })
            .map_err(|e| PyRuntimeError::new_err(e.to_string()));
        let (qua, mask) = det?;
        // allow:clone[python]
        let data = mask.data.clone();
        let (width, height) = (mask.width as usize, mask.height as usize);
        let array = Array2::from_shape_vec((height, width), data)
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;

        let qua = qua
            .into_iter()
            .map(|v| PyQuadrilateral { inner: v })
            .collect();
        Ok((qua, array.into_pyarray(py)))
    }

    fn unload(&mut self) {
        get_runtime().block_on(self.inner.lock().unload())
    }

    fn loaded(&self) -> bool {
        get_runtime().block_on(self.inner.lock().loaded_())
    }
}

#[pymodule]
fn rusty_manga_image_translator(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let red = "\x1b[31m";
    let yellow = "\x1b[33m";
    let reset = "\x1b[0m";

    println!(
        "{}⚠️  Warning: You are using the experimental Python version of this project.{}",
        red, reset
    );
    println!(
            "{}This version is unstable and may break frequently. Please switch to the Rust rewrite for reliability! The rust version is an early release so it might still have some issues. https://github.com/frederik-uni/manga-image-translator-rust{}",
            yellow, reset
        );
    m.add_class::<Session>()?;
    m.add_class::<PyDetector>()?;
    m.add_class::<PyImage>()?;
    m.add_class::<PyDefaultOptions>()?;
    m.add_class::<PyPreprocessorOptions>()?;
    Ok(())
}
