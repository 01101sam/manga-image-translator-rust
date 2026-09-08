use log::{info, Level};
use ndarray::{Array, Array2, IxDyn};
use ort::{
    execution_providers::{
        coreml::{CoreMLComputeUnits, CoreMLModelFormat},
        ArenaExtendStrategy, CUDAExecutionProvider, CoreMLExecutionProvider,
        DirectMLExecutionProvider, ROCmExecutionProvider, TensorRTExecutionProvider,
    },
    session::{
        builder::{GraphOptimizationLevel, SessionBuilder},
        Session,
    },
};

#[derive(Clone, Debug)]
pub enum Providers {
    TensorRT,
    CUDA,
    DirectML,
    CoreML,
    RocM,
}

pub fn all_providers() -> Vec<Providers> {
    vec![
        Providers::CUDA,
        Providers::RocM,
        Providers::TensorRT,
        #[cfg(target_os = "windows")]
        Providers::DirectML,
        #[cfg(any(target_os = "macos", target_os = "ios", target_os = "visionos"))]
        Providers::CoreML,
    ]
}

pub fn gpu_providers() -> Vec<Providers> {
    vec![
        Providers::CUDA,
        Providers::RocM,
        Providers::TensorRT,
        #[cfg(target_os = "windows")]
        Providers::DirectML,
        Providers::RocM,
    ]
}

/// Intra-op threads per session. Only one session runs at a time in the pipeline, so the pool
/// is sized to the machine. On Apple Silicon that means performance cores only: a hard-coded 4
/// left most cores idle and got scheduled onto efficiency cores half the time (2x slower runs),
/// including efficiency-core threads produced ~3x straggler runs, and ORT's own default picked
/// too few threads (M3 Max: 875ms vs 712ms per ctd run). Elsewhere ORT's default (0, physical
/// cores) is used as documented; it has not been measured here.
fn intra_threads() -> usize {
    #[cfg(any(target_os = "macos", target_os = "ios", target_os = "visionos"))]
    {
        extern "C" {
            fn sysctlbyname(
                name: *const std::ffi::c_char,
                oldp: *mut std::ffi::c_void,
                oldlenp: *mut usize,
                newp: *mut std::ffi::c_void,
                newlen: usize,
            ) -> i32;
        }
        let mut cores: i32 = 0;
        let mut len = std::mem::size_of::<i32>();
        let status = unsafe {
            sysctlbyname(
                c"hw.perflevel0.physicalcpu".as_ptr(),
                &mut cores as *mut i32 as *mut std::ffi::c_void,
                &mut len,
                std::ptr::null_mut(),
                0,
            )
        };
        if status == 0 && cores > 0 {
            return cores as usize;
        }
    }
    0
}

pub fn new_session(providers: &[Providers]) -> anyhow::Result<SessionBuilder> {
    Ok(new_session_(
        Session::builder()?,
        providers,
        CoreMLModelFormat::NeuralNetwork,
    )?)
}

/// CoreML's MLProgram format runs more of a graph on the GPU (ctd: 282ms -> 89ms per detect on an
/// M3 Max, same boxes), but its shape propagation aborts the process on dbnet (`mps.concat` shape
/// mismatch at predict time), so models opt in individually after measuring.
pub fn new_session_mlprogram(providers: &[Providers]) -> anyhow::Result<SessionBuilder> {
    Ok(new_session_(
        Session::builder()?,
        providers,
        CoreMLModelFormat::MLProgram,
    )?)
}

pub fn new_session_(
    session_builder: SessionBuilder,
    providers: &[Providers],
    coreml_format: CoreMLModelFormat,
) -> Result<SessionBuilder, ort::Error> {
    let session_builder = session_builder
        .with_logger(Box::new(
            |level: ort::logging::LogLevel,
             category: &str,
             id: &str,
             code_location: &str,
             message: &str| {
                let log_level = match level {
                    ort::logging::LogLevel::Verbose => Level::Trace,
                    ort::logging::LogLevel::Info => Level::Info,
                    ort::logging::LogLevel::Warning => Level::Warn,
                    ort::logging::LogLevel::Error => Level::Error,
                    ort::logging::LogLevel::Fatal => Level::Error,
                };

                log::log!(
                    log_level,
                    "[ORT][{category}][{id}] {message} (at {code_location})"
                );
            },
        ))?
        .with_optimization_level(GraphOptimizationLevel::Level3)?
        .with_intra_threads(intra_threads())?;
    for provider_ in providers {
        let provider = match provider_ {
            Providers::TensorRT => TensorRTExecutionProvider::default()
                .with_device_id(0)
                .build(),
            Providers::CUDA => CUDAExecutionProvider::default()
                .with_device_id(0)
                .with_arena_extend_strategy(ArenaExtendStrategy::SameAsRequested)
                .build(),
            Providers::DirectML => DirectMLExecutionProvider::default()
                .with_device_id(0)
                .build(),
            // GPU only: `All` lets CoreML run fp16 on the Neural Engine, which dropped a text box
            // on ctd and was no faster than the GPU for either detector.
            Providers::CoreML => CoreMLExecutionProvider::default()
                .with_model_cache_dir("models/cache")
                .with_model_format(coreml_format)
                .with_compute_units(CoreMLComputeUnits::CPUAndGPU)
                .build(),
            Providers::RocM => ROCmExecutionProvider::default().with_device_id(0).build(),
        }
        .error_on_failure();
        if let Ok(session_builder) = session_builder
            .clone()
            .with_execution_providers(vec![provider])
        {
            info!("Execution provider {:?} in use", provider_);
            return Ok(session_builder);
        }
    }
    info!("Execution provider CPU in use");
    Ok(session_builder)
}

pub fn dyn_to_2d(arr: Array<f32, IxDyn>) -> Option<Array2<f32>> {
    if arr.ndim() == 2 {
        let shape = arr.shape();
        let (rows, cols) = (shape[0], shape[1]);

        arr.into_shape((rows, cols)).ok()
    } else {
        None
    }
}
