use std::{collections::HashMap, ops::Deref, sync::Arc};

use image::imageops::crop_imm;
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

impl NativeOCR {}

#[async_trait::async_trait]
impl ModelLoad for NativeOCR {
    impl_model_load_helpers!(model, OcrEngine);

    async fn reload(&self) -> anyhow::Result<ModelRead<'_, Self::T>> {
        let engine = OcrEngine::new(OcrProvider::Auto).unwrap().with_options(
            OcrOptions::default().languages(vec![
                Language::Chinese,
                Language::Japanese,
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
            let bbox = area.lock().aabb();
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
                Mask::from(crop_imm(&grayscale, x, y, w, h).to_image())
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

    #[tokio::test]
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
        assert_eq!(v[0].text, "そうだなあ・・・");
        assert_eq!(v.len(), 2);
    }
}
