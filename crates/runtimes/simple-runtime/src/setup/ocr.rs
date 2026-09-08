use std::{collections::HashMap, sync::Arc};

use base_util::onnx::{all_providers, gpu_providers};
use strum::IntoEnumIterator;

use crate::settings::OCR;

pub type OcrType = Box<dyn interface_ocr::Ocr + Send + Sync>;

pub struct OCRs(HashMap<OCR, OcrType>);
impl OCRs {
    pub fn get(&self, ocr: OCR) -> &OcrType {
        self.0.get(&ocr).expect("OCR not registered")
    }
    pub fn new(max_batch_size: usize) -> Self {
        let mut items = HashMap::new();
        let providers = Arc::new(all_providers());
        for key in OCR::iter() {
            let ocr = match key {
                OCR::MangaOcr => {
                    // allow:clone
                    Box::new(manga_ocr::MangaOCR::new(providers.clone(), 256)) as OcrType
                }
                // allow:clone
                OCR::Native => Box::new(native::NativeOCR::default()) as OcrType,
                // allow:clone
                OCR::Ctc48px => {
                    Box::new(ctc_48px::Ctc48pxOcr::new(providers.clone(), max_batch_size))
                        as OcrType
                }
                // allow:clone
                OCR::Ocr48px => Box::new(ocr_48px::Ocr48px::new(
                    providers.clone(),
                    256,
                    max_batch_size,
                )) as OcrType,
            };
            items.insert(key, ocr);
        }
        OCRs(items)
    }
}
