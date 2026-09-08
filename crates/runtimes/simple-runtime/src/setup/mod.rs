mod colorizer;
mod detector;
mod inpainter;
mod ocr;
mod translator;
mod upscaler;
pub use detector::DetectorType;
pub use detector::Detectors;
use interface_translator::LangIdDetector;
pub use upscaler::UpscalerType;
pub use upscaler::Upscalers;

use crate::settings::Colorizer;
use crate::settings::Detector;
use crate::settings::Inpainter;
use crate::settings::Translator;
use crate::settings::Upscaler;
use crate::settings::OCR;
use crate::setup::colorizer::ColorizerType;
use crate::setup::colorizer::Colorizers;
use crate::setup::inpainter::InpainterType;
use crate::setup::inpainter::Inpainters;
use crate::setup::ocr::OCRs;
use crate::setup::ocr::OcrType;
use crate::setup::translator::TranslatorType;
use crate::setup::translator::Translators;

pub struct Models {
    upscalers: Upscalers,
    colorizers: Colorizers,
    detectors: Detectors,
    ocrs: OCRs,
    translators: Translators,
    inpainters: Inpainters,
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
    pub async fn get_translator(
        &mut self,
        translator: Translator,
    ) -> anyhow::Result<&mut TranslatorType> {
        self.translators.get(translator).await
    }
    pub fn get_inpainter(&self, inpainter: Inpainter) -> &InpainterType {
        self.inpainters.get(inpainter)
    }
    pub async fn new(
        max_batch_size_upscaler: usize,
        max_batch_size_ocr: usize,
        fast: bool,
        cuda: bool,
    ) -> Self {
        //TODO: providers based on input
        Models {
            lang_detector: LangIdDetector::new().unwrap(),
            detectors: Detectors::new(),
            colorizers: Colorizers::new(),
            upscalers: Upscalers::new(max_batch_size_upscaler, fast),
            inpainters: Inpainters::new(),
            ocrs: OCRs::new(max_batch_size_ocr),
            translators: Translators::new(cuda),
        }
    }
}
