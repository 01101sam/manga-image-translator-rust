use std::{cmp::Ordering, ops::Deref, path::PathBuf, sync::Arc};

use base_util::onnx::{new_session, Providers};
use image::imageops::crop_imm;
use interface_detector::textlines::Quadrilateral;
use interface_image::{ImageOp, Mask};
use interface_model::{
    impl_model_helpers, impl_model_load_helpers, Model, ModelLoad, ModelRead, ModelSource,
    ModelWrap,
};
use interface_ocr::{OcrOptions, QuadrilateralInfo};
use maplit::hashmap;
use ndarray::{s, stack, Array4, ArrayView2, Axis};
use ort::{inputs, session::Session, value::Tensor};
use parking_lot::Mutex;
use tokio::task::spawn_blocking;
use util::spawn_blocking;

pub struct MangaOCRModels {
    enc: std::sync::Mutex<Session>,
    dec: std::sync::Mutex<Session>,
    vocab: Vec<String>,
}

impl MangaOCRModels {
    fn new(
        enc: PathBuf,
        dec: PathBuf,
        vocab: PathBuf,
        providers: &[Providers],
    ) -> anyhow::Result<Self> {
        // CoreML loses on both graphs (30 lines on an M3 Max: 7.5s -> 2.0s). The NeuralNetwork
        // format shatters the ViT encoder into 87 partitions (3.8s vs 1.3s per page on the CPU) and
        // the greedy decoder into 34 (3.7s vs 0.55s); the MLProgram format fails to compile the
        // encoder ("ios18.conv: output size is too small").
        let no_coreml: Vec<_> = providers
            .iter()
            .filter(|p| !matches!(p, Providers::CoreML))
            .cloned()
            .collect();
        let enc = std::sync::Mutex::new(new_session(&no_coreml)?.commit_from_file(&enc)?);
        let dec = std::sync::Mutex::new(new_session(&no_coreml)?.commit_from_file(&dec)?);

        let vocab = std::fs::read_to_string(vocab)?
            .lines()
            .map(|s| s.to_string())
            .collect::<Vec<_>>();
        Ok(Self { enc, dec, vocab })
    }
}

pub struct MangaOCR {
    models: ModelWrap<MangaOCRModels>,
    providers: Arc<Vec<Providers>>,
    max_length: usize,
}

impl MangaOCR {
    pub fn new(providers: Arc<Vec<Providers>>, max_length: usize) -> Self {
        Self {
            models: Default::default(),
            providers,
            max_length,
        }
    }
}

#[async_trait::async_trait]
impl ModelLoad for MangaOCR {
    impl_model_load_helpers!(models, MangaOCRModels);

    async fn reload(&self) -> anyhow::Result<ModelRead<'_, Self::T>> {
        let enc = self.download_model("enc", "encoder_model.onnx").await?;
        let dec = self.download_model("dec", "decoder_model.onnx").await?;
        let voc = self.download_model("vocab", "vocab.txt").await?;
        *self.models.write().await = Some(MangaOCRModels::new(enc, dec, voc, &self.providers)?);
        Ok(ModelRead::new(self.models.read().await))
    }
}

#[async_trait::async_trait]
impl Model for MangaOCR {
    impl_model_helpers!("ocr", "manga-ocr", models);

    fn models(&self) -> std::collections::HashMap<&'static str, interface_model::ModelSource> {
        hashmap! {
            "enc" => ModelSource { url: "https://huggingface.co/mayocream/manga-ocr-onnx/resolve/main/encoder_model.onnx?download=true", hash: "###" },
            "dec" => ModelSource { url: "https://huggingface.co/mayocream/manga-ocr-onnx/resolve/main/decoder_model.onnx?download=true", hash: "###" },
            "vocab" => ModelSource { url: "https://huggingface.co/mayocream/manga-ocr-onnx/resolve/main/vocab.txt?download=true", hash: "###" },
        }
    }
}

impl MangaOCR {
    async fn detect_patch(
        &self,
        image: Mask,
        area: Arc<Mutex<Quadrilateral>>,
        img_processor: &Arc<dyn ImageOp + Send + Sync>,
    ) -> anyhow::Result<QuadrilateralInfo> {
        self.load().await?;
        let sos_idnex = 2;
        let eos_index = 3;
        let special_tokens = 5;
        // allow:clone[arc]
        let pre = preprocessor(image, img_processor.clone()).await?;

        let t = Tensor::from_array(pre)?;
        let models = self.load().await?;
        let models = models.deref();

        // Synchronous `Session::run` on purpose; see `dbnet` for why `RunAsync` is avoided.
        let mut enc = models.enc.lock().expect("session mutex poisoned");
        let out = enc.run(inputs! {"pixel_values" => t})?;
        let hs = &out[0];
        let mut dec = models.dec.lock().expect("session mutex poisoned");

        let mut token_ids: Vec<i64> = vec![sos_idnex];
        for _ in 0..self.max_length {
            let input = ArrayView2::from_shape((1, token_ids.len()), &token_ids)?;
            let t = Tensor::from_array(input.to_owned())?;

            let out = dec.run(inputs! {
                "encoder_hidden_states" => hs,
                "input_ids" => t,
            })?;
            let logits = out["logits"].try_extract_array::<f32>()?;
            let v = logits.slice(s![0, -1, ..]);
            let token_id = v
                .iter()
                .enumerate()
                .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(Ordering::Equal))
                .map(|v| v.0)
                .unwrap_or_default();

            token_ids.push(token_id as i64);
            if token_id as i64 == eos_index {
                break;
            }
        }
        Ok(QuadrilateralInfo {
            text: token_ids
                .into_iter()
                .filter(|&id| id >= special_tokens)
                .map(|v| models.vocab[v as usize].to_owned())
                .collect::<String>(),
            fg: None,
            bg: None,
            // allow:clone[arc]
            pos: area.clone(),
            prob: 1.0,
        })
    }
}
#[async_trait::async_trait]
impl interface_ocr::Ocr for MangaOCR {
    async fn detect(
        &self,
        image: &interface_image::RawImage,
        areas: &[Arc<Mutex<Quadrilateral>>],
        options: OcrOptions,
        img_processor: &Arc<dyn ImageOp + Send + Sync>,
    ) -> anyhow::Result<Vec<interface_ocr::QuadrilateralInfo>> {
        let mut texts = vec![];
        let grayscale =
            spawn_blocking!(|| Ok::<_, anyhow::Error>(image.clone().to_image()?.to_luma8()))??;

        for (i, area) in areas.iter().enumerate() {
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

async fn preprocessor(
    img: Mask,
    img_processor: Arc<dyn ImageOp + Send + Sync>,
) -> anyhow::Result<Array4<f32>> {
    //"resample": 2,"size": 224
    spawn_blocking(move || {
        let resized = img_processor.resize_mask(
            img.view(),
            224,
            224,
            interface_image::Interpolation::Bilinear,
        )?;
        let img = resized
            .as_nd()?
            .mapv(|pixel| pixel as f32 / 255.0 * 2.0 - 1.0);

        Ok(base_util::ndarray_utils::to_contiguous2(
            stack(Axis(0), &[img.view(), img.view(), img.view()])?.insert_axis(Axis(0)),
        ))
    })
    .await
    .unwrap()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use base_util::onnx::all_providers;
    use interface_detector::textlines::Quadrilateral;
    use interface_image::{CpuImageProcessor, ImageOp, RawImage};
    use interface_ocr::Ocr as _;
    use parking_lot::Mutex;

    use crate::MangaOCR;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ocr_test() {
        let img = RawImage::new("./imgs/232265329-6a560438-e887-4f7f-b6a1-a61b8648f781.png")
            .expect("Failed to load image");
        let mocr = MangaOCR::new(Arc::new(all_providers()), 255);
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
        let mut v = mocr
            .detect(&Arc::new(img), &inp, Default::default(), &ip)
            .await
            .unwrap();
        v.sort_by_key(|a| a.text.len());
        assert_eq!(v[0].pos.lock().pts()[0].x, 76);
        assert_eq!(v[0].text, "ふふっ、");
        assert_eq!(v[1].text, "そうだなあ・・・");
        assert_eq!(v.len(), 2);
    }
}
