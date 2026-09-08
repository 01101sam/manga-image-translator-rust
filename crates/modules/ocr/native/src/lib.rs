use std::{collections::HashMap, ops::Deref, sync::Arc};

use image::{
    imageops::{crop_imm, replace},
    GrayImage, Luma,
};
use interface_detector::textlines::Quadrilateral;
use interface_image::{ImageOp, Mask, RawImage};
use interface_model::{
    impl_model_helpers, impl_model_load_helpers, Model, ModelLoad, ModelRead, ModelWrap,
};
use interface_ocr::{Ocr, QuadrilateralInfo};
use parking_lot::Mutex;
use uni_ocr::{Language, OcrEngine, OcrOptions, OcrProvider};
use util::spawn_blocking;

#[derive(Default)]
pub struct NativeOCR {
    model: ModelWrap<OcrEngine>,
}

/// Blank margin left around every glyph cell by [`reflow_vertical`].
const CELL_GAP: u32 = 6;

/// Rewrite a vertical (tategaki) line as a single horizontal row of upright glyph cells.
///
/// Vision's text request only ever groups glyphs into horizontal lines: fed a column of CJK
/// glyphs it reports no observation at all, whatever the revision, language set or padding.
/// Cells are cut on the blank rows between glyphs, then merged/subdivided so that each one
/// holds about one glyph, which keeps reading order (top-to-bottom becomes left-to-right).
fn reflow_vertical(src: &GrayImage) -> GrayImage {
    let (w, h) = (src.width(), src.height());
    let glyph = w.max(2);
    // a row counts as blank when only a few stray dark pixels sit on it
    let inked: Vec<bool> = (0..h)
        .map(|y| {
            (0..w).filter(|&x| src.get_pixel(x, y).0[0] < 128).count() as u32 > (glyph / 16).max(1)
        })
        .collect();

    let mut cuts = vec![0];
    let mut y = 0;
    while y < h {
        let blank_start = y;
        while y < h && !inked[y as usize] {
            y += 1;
        }
        let mid = (blank_start + y) / 2;
        let interior = blank_start > 0 && y < h;
        // skip boundaries that would carve out a fragment smaller than a glyph
        if interior && mid - cuts[cuts.len() - 1] >= glyph / 2 && h - mid >= glyph / 2 {
            cuts.push(mid);
        }
        while y < h && inked[y as usize] {
            y += 1;
        }
    }
    cuts.push(h);

    // glyphs touching vertically share a cell; split those back into square-ish cells
    let cells: Vec<(u32, u32)> = cuts
        .windows(2)
        .flat_map(|c| {
            let (top, len) = (c[0], c[1] - c[0]);
            let parts = ((len as f32 / glyph as f32).round() as u32).max(1);
            (0..parts).map(move |k| (top + k * len / parts, top + (k + 1) * len / parts))
        })
        .collect();

    let cell_h = cells.iter().map(|c| c.1 - c.0).max().unwrap_or(h);
    let pitch = w + CELL_GAP;
    let mut out = GrayImage::from_pixel(
        cells.len() as u32 * pitch + CELL_GAP,
        cell_h + 2 * CELL_GAP,
        Luma([255]),
    );
    for (k, (top, bottom)) in cells.into_iter().enumerate() {
        let cell = crop_imm(src, 0, top, w, bottom - top).to_image();
        replace(
            &mut out,
            &cell,
            (CELL_GAP + k as u32 * pitch) as i64,
            CELL_GAP as i64,
        );
    }
    out
}

#[async_trait::async_trait]
impl ModelLoad for NativeOCR {
    impl_model_load_helpers!(model, OcrEngine);

    async fn reload(&self) -> anyhow::Result<ModelRead<'_, Self::T>> {
        // Vision reads this list as a priority order, not a set: with zh-Hans leading it drops
        // nearly every kana line, while ja-JP leading leaves Chinese pages unchanged.
        let engine = OcrEngine::new(OcrProvider::Auto).unwrap().with_options(
            OcrOptions::default().languages(vec![
                Language::Japanese,
                Language::Chinese,
                Language::Korean,
                Language::English,
            ]),
        );
        *self.model.write().await = Some(engine);
        Ok(self.get_model().await.expect("model loaded"))
    }
}

#[async_trait::async_trait]
impl Model for NativeOCR {
    impl_model_helpers!("ocr", "native", model);

    fn models(&self) -> std::collections::HashMap<&'static str, interface_model::ModelSource> {
        HashMap::new()
    }
}

#[async_trait::async_trait]
impl Ocr for NativeOCR {
    async fn detect(
        &self,
        image: &RawImage,
        areas: &[Arc<Mutex<Quadrilateral>>],
        options: interface_ocr::OcrOptions,
        img_processor: &Arc<dyn ImageOp + Send + Sync>,
    ) -> anyhow::Result<Vec<interface_ocr::QuadrilateralInfo>> {
        let mut texts = vec![];
        let grayscale =
            spawn_blocking!(|| Ok::<_, anyhow::Error>(image.clone().to_image()?.to_luma8()))??;

        for (i, area) in areas.into_iter().enumerate() {
            let (bbox, vertical) = {
                let q = area.lock();
                (q.aabb(), q.vertical())
            };
            let Some((x, y, w, h)) = util::resize::clamp_crop(
                bbox.x,
                bbox.y,
                bbox.x.saturating_add(bbox.w),
                bbox.y.saturating_add(bbox.h),
                grayscale.width() as i64,
                grayscale.height() as i64,
            ) else {
                continue;
            };
            let img = spawn_blocking!(|| {
                let line = crop_imm(&grayscale, x, y, w, h).to_image();
                Mask::from(if vertical {
                    reflow_vertical(&line)
                } else {
                    line
                })
            })?;
            if let Some(v) = &options.debug_path {
                img.clone()
                    .to_image()?
                    .save(v.join(format!("patch_{i}_0.png")))?
            }

            // allow:clone[arc]
            texts.push(self.detect_patch(img, area.clone(), img_processor).await?);
        }
        Ok(texts)
    }
}

impl NativeOCR {
    async fn detect_patch(
        &self,
        sliced_image: interface_image::Mask,
        area: Arc<Mutex<Quadrilateral>>,
        _: &Arc<dyn interface_image::ImageOp + Send + Sync>,
    ) -> anyhow::Result<interface_ocr::QuadrilateralInfo> {
        let model = self.load().await?;
        let model = model.deref();
        let image = tokio::task::spawn_blocking(move || {
            image::DynamicImage::from(sliced_image.to_image().unwrap())
        })
        .await?;

        let (result, _, prob) = model.recognize_image(&image).await?;
        Ok(QuadrilateralInfo {
            text: result,
            fg: None,
            bg: None,
            pos: area,
            prob: prob.unwrap_or(1.0),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use interface_detector::textlines::Quadrilateral;
    use interface_image::{CpuImageProcessor, ImageOp, RawImage};
    use interface_ocr::Ocr as _;
    use parking_lot::Mutex;

    use crate::NativeOCR;

    // `detect` reaches Vision through `spawn_blocking!`, which needs the multi-threaded runtime
    #[tokio::test(flavor = "multi_thread")]
    async fn ocr_test() {
        let img = RawImage::new("./imgs/232265329-6a560438-e887-4f7f-b6a1-a61b8648f781.png")
            .expect("Failed to load image");
        let mocr = NativeOCR::default();
        let inp = vec![
            Arc::new(Mutex::new(Quadrilateral::new(
                vec![(208, 4), (246, 4), (246, 192), (208, 192)],
                1.0,
            ))),
            Arc::new(Mutex::new(Quadrilateral::new(
                vec![(76, 1788), (128, 1788), (128, 1930), (76, 1930)],
                1.0,
            ))),
        ];
        let ip = Arc::new(CpuImageProcessor::default()) as Arc<dyn ImageOp + Send + Sync>;
        let v = mocr
            .detect(&Arc::new(img), &inp, Default::default(), &ip)
            .await
            .unwrap();
        assert_eq!(v.len(), 2);
        // Vision spells the trailing 「・・・」 with a punctuation mark of its own choosing
        assert!(
            v[0].text.starts_with("そうだなあ"),
            "line 0 was {:?}",
            v[0].text
        );
        // a vertical line Vision failed to group comes back as an empty string at zero confidence
        assert!(
            v.iter().all(|l| !l.text.is_empty() && l.prob > 0.0),
            "{:?}",
            v.iter().map(|l| (&l.text, l.prob)).collect::<Vec<_>>()
        );
    }
}
