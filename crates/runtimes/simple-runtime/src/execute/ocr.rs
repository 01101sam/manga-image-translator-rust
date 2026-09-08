use std::{fs::create_dir_all, path::PathBuf, sync::Arc};

use interface_detector::textlines::Quadrilateral;
use interface_image::RawImage;
use interface_ocr::{OcrOptions, QuadrilateralInfo};
use log::info;
use parking_lot::Mutex;

use crate::{execute::ImageProcessor, settings::OCRSettings, setup::Models};

impl Models {
    pub async fn run_ocr(
        &self,
        img: &RawImage,
        areas: &[Arc<Mutex<Quadrilateral>>],
        config: &OCRSettings,
        debug_path: &Option<PathBuf>,
        ip: &ImageProcessor,
    ) -> anyhow::Result<Vec<QuadrilateralInfo>> {
        let debug_path = if let Some(debug_path) = debug_path {
            let p = debug_path.join("ocr_patches");
            create_dir_all(&p)?;
            Some(p)
        } else {
            None
        };
        info!("Run OCR: {:?}", config.ocr);
        let textlines = self
            .get_ocr(config.ocr)
            .detect(img, areas, OcrOptions { debug_path }, ip)
            .await?;
        let ignore = config.post_processing.ignore_bubble;
        if ignore < 1 || ignore > 50 {
            return Ok(textlines);
        }
        let mut kept = Vec::with_capacity(textlines.len());
        for line in textlines {
            let pos = line.pos.lock();
            let (x1, y1, x2, y2) = pos.xyxy();
            drop(pos);
            let x1 = x1.max(0) as u32;
            let y1 = y1.max(0) as u32;
            let x2 = (x2.max(0) as u32).min(img.width as u32);
            let y2 = (y2.max(0) as u32).min(img.height as u32);
            if x2 <= x1 || y2 <= y1 {
                kept.push(line);
                continue;
            }
            let cropped = img.clone().to_image()?.crop_imm(x1, y1, x2 - x1, y2 - y1);
            let raw = RawImage::from(cropped);
            let mat = raw.as_opencv_mat()?.clone_pointee();
            if mask_refinement::is_ignore((raw.width, raw.height), &mat, ignore)? {
                continue;
            }
            kept.push(line);
        }
        Ok(kept)
    }
}
