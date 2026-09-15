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

const EP_ASSIGNMENT_REPORT: &str = "Some nodes were not assigned to the preferred execution providers which may or may not have an negative impact on performance. e.g. ORT explicitly assigns shape related ops to CPU to improve perf.";
const CTC_CLASSIFIER_FALLBACKS: [&str; 2] = [
    "CoreML does not support input dim > 16384. Input:onnx::MatMul_1371, shape: {320,19264}",
    "CoreML does not support input dim > 16384. Input:char_pred.bias, shape: {19264}",
];

fn expected_coreml_warning(category: &str, message: &str) -> bool {
    let Some((source, function)) = category.rsplit_once(' ') else {
        return false;
    };
    match function {
        "GetCapability" if source.starts_with("coreml_execution_provider.cc:") => {
            let Some(report) = message.strip_prefix(
                "CoreMLExecutionProvider::GetCapability, number of partitions supported by CoreML: ",
            ) else {
                return false;
            };
            let Some((partitions, report)) = report.split_once(" number of nodes in the graph: ")
            else {
                return false;
            };
            let Some((nodes, supported)) =
                report.split_once(" number of nodes supported by CoreML: ")
            else {
                return false;
            };
            // 完全回退 CPU 或无法识别的统计保留警告，避免掩盖加速器失效。
            matches!(
                (partitions.parse::<usize>(), nodes.parse::<usize>(), supported.parse::<usize>()),
                (Ok(partitions), Ok(nodes), Ok(supported)) if partitions > 0 && supported > 0 && supported <= nodes
            )
        }
        "IsInputSupported" if source.starts_with("helper.cc:") => {
            CTC_CLASSIFIER_FALLBACKS.contains(&message)
        }
        "VerifyEachNodeIsAssignedToAnEp" if source.starts_with("session_state.cc:") => {
            message == EP_ASSIGNMENT_REPORT
        }
        _ => false,
    }
}

fn map_ort_log_level(
    level: ort::logging::LogLevel,
    category: &str,
    message: &str,
    coreml_active: bool,
) -> Level {
    match level {
        ort::logging::LogLevel::Verbose => Level::Trace,
        ort::logging::LogLevel::Info => Level::Info,
        ort::logging::LogLevel::Warning
            if coreml_active && expected_coreml_warning(category, message) =>
        {
            Level::Debug
        }
        ort::logging::LogLevel::Warning => Level::Warn,
        ort::logging::LogLevel::Error | ort::logging::LogLevel::Fatal => Level::Error,
    }
}

pub fn new_session_(
    session_builder: SessionBuilder,
    providers: &[Providers],
    coreml_format: CoreMLModelFormat,
) -> Result<SessionBuilder, ort::Error> {
    let mut session_builder = session_builder
        .with_optimization_level(GraphOptimizationLevel::Level3)?
        .with_intra_threads(intra_threads())?;
    let mut selected_provider = None;
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
        if let Ok(configured) = session_builder
            .clone()
            .with_execution_providers(vec![provider])
        {
            session_builder = configured;
            selected_provider = Some(provider_);
            break;
        }
    }
    match selected_provider {
        Some(provider) => info!("Execution provider {:?} in use", provider),
        None => info!("Execution provider CPU in use"),
    }
    // 以注册成功的提供者为准，候选列表包含 CoreML 不代表会话使用它。
    let coreml_active = matches!(selected_provider, Some(Providers::CoreML));
    session_builder.with_logger(Box::new(
        move |level: ort::logging::LogLevel,
              category: &str,
              id: &str,
              code_location: &str,
              message: &str| {
            log::log!(
                map_ort_log_level(level, category, message, coreml_active),
                "[ORT][{category}][{id}] {message} (at {code_location})"
            );
        },
    ))
}

pub fn dyn_to_2d(arr: Array<f32, IxDyn>) -> Option<Array2<f32>> {
    if arr.ndim() == 2 {
        let shape = arr.shape();
        let (rows, cols) = (shape[0], shape[1]);

        arr.into_shape_with_order((rows, cols)).ok()
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use log::Level;
    use ort::logging::LogLevel;

    use super::map_ort_log_level;

    #[test]
    fn demotes_only_exact_coreml_partition_reports() {
        for (category, message) in [
            (
                "coreml_execution_provider.cc:113 GetCapability",
                "CoreMLExecutionProvider::GetCapability, number of partitions supported by CoreML: 33 number of nodes in the graph: 1123 number of nodes supported by CoreML: 689",
            ),
            (
                "helper.cc:83 IsInputSupported",
                "CoreML does not support input dim > 16384. Input:char_pred.bias, shape: {19264}",
            ),
            (
                "helper.cc:83 IsInputSupported",
                "CoreML does not support input dim > 16384. Input:onnx::MatMul_1371, shape: {320,19264}",
            ),
            (
                "session_state.cc:1280 VerifyEachNodeIsAssignedToAnEp",
                "Some nodes were not assigned to the preferred execution providers which may or may not have an negative impact on performance. e.g. ORT explicitly assigns shape related ops to CPU to improve perf.",
            ),
        ] {
            assert_eq!(map_ort_log_level(LogLevel::Warning, category, message, true), Level::Debug);
            assert_eq!(map_ort_log_level(LogLevel::Warning, category, message, false), Level::Warn);
        }
    }

    #[test]
    fn keeps_unknown_coreml_fallbacks_visible() {
        for (category, message) in [
            (
                "coreml_execution_provider.cc:113 GetCapability",
                "CoreMLExecutionProvider::GetCapability, number of partitions supported by CoreML: 0 number of nodes in the graph: 184 number of nodes supported by CoreML: 0",
            ),
            (
                "coreml_execution_provider.cc:113 GetCapability",
                "CoreMLExecutionProvider::GetCapability, number of partitions supported by CoreML: unknown number of nodes in the graph: 184 number of nodes supported by CoreML: 173",
            ),
            (
                "helper.cc:83 IsInputSupported",
                "CoreML does not support input dim > 16384. Input:image, shape: {1,3,20000,48}",
            ),
            (
                "coreml_execution_provider.cc:113 GetCapability",
                "CoreMLExecutionProvider::GetCapability failed to compile subgraph",
            ),
            (
                "session_state.cc:1280 VerifyEachNodeIsAssignedToAnEp",
                "Some nodes were not assigned to the preferred execution providers because CoreML failed",
            ),
            (
                "cuda_execution_provider.cc:1 GetCapability",
                "CoreMLExecutionProvider::GetCapability, number of partitions supported by CoreML: 1 number of nodes in the graph: 184 number of nodes supported by CoreML: 173",
            ),
            (
                "helper.cc:83 FailedIsInputSupported",
                "CoreML does not support input dim > 16384. Input:char_pred.bias, shape: {19264}",
            ),
        ] {
            assert_eq!(map_ort_log_level(LogLevel::Warning, category, message, true), Level::Warn);
        }
    }

    #[test]
    fn keeps_non_warning_levels_unchanged() {
        for coreml_active in [false, true] {
            for (level, expected) in [
                (LogLevel::Verbose, Level::Trace),
                (LogLevel::Info, Level::Info),
                (LogLevel::Error, Level::Error),
                (LogLevel::Fatal, Level::Error),
            ] {
                assert_eq!(
                    map_ort_log_level(
                        level,
                        "helper.cc:83 IsInputSupported",
                        "CoreML does not support input dim > 16384. Input:char_pred.bias, shape: {19264}",
                        coreml_active,
                    ),
                    expected
                );
            }
        }
    }
}
