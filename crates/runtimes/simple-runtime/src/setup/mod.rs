mod colorizer;
mod detector;
mod inpainter;
mod ocr;
mod translator;
mod upscaler;
pub use detector::DetectorType;
pub use detector::Detectors;
use interface_translator::LangIdDetector;
pub use translator::create_translator;
pub use upscaler::UpscalerType;
pub use upscaler::Upscalers;

use std::sync::Arc;

use base_util::onnx::all_providers;
use pixai_tagger::PixaiTagger;

use crate::settings::Colorizer;
use crate::settings::Detector;
use crate::settings::Inpainter;
use crate::settings::Upscaler;
use crate::settings::OCR;
use crate::setup::colorizer::ColorizerType;
use crate::setup::colorizer::Colorizers;
use crate::setup::inpainter::InpainterType;
use crate::setup::inpainter::Inpainters;
use crate::setup::ocr::OCRs;
use crate::setup::ocr::OcrType;

pub struct Models {
    upscalers: Upscalers,
    colorizers: Colorizers,
    detectors: Detectors,
    ocrs: OCRs,
    inpainters: Inpainters,
    /// `None` once tagging failed (model not downloadable): the pipeline then runs without tags.
    pub tagger: Option<PixaiTagger>,
    pub lang_detector: LangIdDetector,
}

impl Models {
    pub fn get_upscaler(&self, upscaler: Upscaler) -> &UpscalerType {
        self.upscalers.get(upscaler)
    }
    pub fn get_colorizer(&self, colorizer: Colorizer) -> &ColorizerType {
        self.colorizers.get(colorizer)
    }
    pub fn get_detector(&self, detector: Detector) -> &DetectorType {
        self.detectors.get(detector)
    }
    pub fn get_ocr(&self, ocr: OCR) -> &OcrType {
        self.ocrs.get(ocr)
    }
    pub fn get_inpainter(&self, inpainter: Inpainter) -> &InpainterType {
        self.inpainters.get(inpainter)
    }
    pub async fn new(
        max_batch_size_upscaler: usize,
        max_batch_size_ocr: usize,
        fast: bool,
        _cuda: bool,
    ) -> Self {
        Models {
            lang_detector: LangIdDetector::new().unwrap(),
            detectors: Detectors::new(),
            colorizers: Colorizers::new(),
            upscalers: Upscalers::new(max_batch_size_upscaler, fast),
            inpainters: Inpainters::new(),
            tagger: Some(PixaiTagger::new(Arc::new(all_providers()))),
            ocrs: OCRs::new(max_batch_size_ocr),
        }
    }
}
