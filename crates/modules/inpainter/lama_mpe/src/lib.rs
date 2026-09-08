mod mpe;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use base_util::onnx::{new_session, new_session_mlprogram, Providers};
use interface_image::{ImageOp, RawImageCow};
use interface_inpainter::{Inpainter, InpainterOptions};
use interface_model::{
    impl_model_load_helpers, Model, ModelLoad, ModelRead, ModelSource, ModelWrap,
};
use log::warn;
use maplit::hashmap;
use ndarray::{s, Array2, Array3, ArrayView4, Axis};
use ort::{inputs, session::Session, value::Tensor};
use util::{
    lama::{lama_add_border, lama_pad_canvas, lama_resize_image},
    spawn_blocking,
};

/// A fixed-canvas export of model.onnx (`scripts/lama_mpe_static_export.py`), which CoreML runs as a
/// single partition: 862 ms vs 9.5 s on the CPU EP for a 1488x2048 page on an M3 Max. The dynamic
/// model shatters into 300+ CoreML partitions and ends up slower than the CPU EP.
struct Tier {
    width: u16,
    height: u16,
    file: &'static str,
    url: &'static str,
}

/// Ordered by area, so the first tier that covers a canvas is the cheapest (latency is linear in area).
/// The portrait tier is there because manga pages resized to `inpainting_size` are ~1450-1500 wide.
const TIERS: [Tier; 3] = [
    Tier {
        width: 1024,
        height: 1024,
        file: "model_static_1024x1024.onnx",
        url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/lama_mpe/model_static_1024x1024.onnx",
    },
    Tier {
        width: 1536,
        height: 2048,
        file: "model_static_1536x2048.onnx",
        url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/lama_mpe/model_static_1536x2048.onnx",
    },
    Tier {
        width: 2048,
        height: 2048,
        file: "model_static_2048x2048.onnx",
        url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/lama_mpe/model_static_2048x2048.onnx",
    },
];

pub struct LamaLargeInpainter {
    /// Dynamic-shape model.onnx: serves every canvas the tiers do not, never through CoreML.
    model: ModelWrap<Mutex<Session>>,
    /// Tier sessions by file, created on first use. `None` records a tier that failed to load (file
    /// absent and not downloadable, or rejected by the session) so it is not retried.
    tiers: Mutex<HashMap<&'static str, Option<Arc<Mutex<Session>>>>>,
    providers: Arc<Vec<Providers>>,
}

impl LamaLargeInpainter {
    pub fn new(providers: Arc<Vec<Providers>>) -> Self {
        Self {
            model: Default::default(),
            tiers: Default::default(),
            providers,
        }
    }

    /// The cheapest loadable tier covering a `width` x `height` canvas. `None` when CoreML was not
    /// requested (elsewhere the dynamic model already runs whole on the GPU, and the fixed canvas
    /// would only add padding work and downloads), when no tier is large enough, or when every
    /// fitting tier failed to load; the dynamic model then takes the image.
    async fn tier_session(
        &self,
        width: u16,
        height: u16,
    ) -> Option<(&'static Tier, Arc<Mutex<Session>>)> {
        if !self
            .providers
            .iter()
            .any(|p| matches!(p, Providers::CoreML))
        {
            return None;
        }
        for tier in TIERS
            .iter()
            .filter(|t| t.width >= width && t.height >= height)
        {
            let cached = self
                .tiers
                .lock()
                .expect("tier map poisoned")
                .get(tier.file)
                .cloned();
            let session = match cached {
                Some(slot) => slot,
                None => {
                    let loaded = self
                        .load_tier(tier)
                        .await
                        .map_err(|e| {
                            warn!("lama_mpe {} unavailable, falling back: {e:#}", tier.file)
                        })
                        .ok();
                    self.tiers
                        .lock()
                        .expect("tier map poisoned")
                        .insert(tier.file, loaded.clone());
                    loaded
                }
            };
            if let Some(session) = session {
                return Some((tier, session));
            }
        }
        None
    }

    async fn load_tier(&self, tier: &Tier) -> anyhow::Result<Arc<Mutex<Session>>> {
        let path = self.download_model(tier.file, tier.file).await?;
        let session = new_session_mlprogram(&self.providers)?.commit_from_file(&path)?;
        Ok(Arc::new(Mutex::new(session)))
    }
}

#[async_trait::async_trait]
impl ModelLoad for LamaLargeInpainter {
    impl_model_load_helpers!(model, Mutex<Session>);
    async fn reload(&self) -> anyhow::Result<ModelRead<'_, Self::T>> {
        let p = self.download_model("model", "model.onnx").await?;
        // CoreML cannot host this graph's FFC spectral path (Einsum/Range/Sin/Cos have no CoreML
        // builder, every Reshape takes a runtime shape from the dynamic H/W), so it shatters the
        // model into 300+ partitions and runs 3-4x slower than the CPU EP (30-35s vs 8-10s on an
        // M3 Max, either model format). The fixed-canvas tiers exist for that case; this session
        // only runs what they do not cover.
        let providers: Vec<_> = self
            .providers
            .iter()
            .filter(|p| !matches!(p, Providers::CoreML))
            .cloned()
            .collect();
        let session = new_session(&providers)?.commit_from_file(&p)?;
        *self.model.write().await = Some(Mutex::new(session));
        Ok(self.get_model().await.expect("set before"))
    }
}

#[async_trait::async_trait]
impl Model for LamaLargeInpainter {
    fn name(&self) -> &'static str {
        "lama_mpe"
    }

    fn kind(&self) -> &'static str {
        "inpainter"
    }

    fn models(&self) -> HashMap<&'static str, ModelSource> {
        let mut models = hashmap! {"model" => ModelSource{ url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/lama_mpe/model.onnx", hash: "###" }};
        for tier in &TIERS {
            models.insert(
                tier.file,
                ModelSource {
                    url: tier.url,
                    hash: "###",
                },
            );
        }
        models
    }

    async fn unload(&self) {
        *self.model.write().await = None;
        self.tiers.lock().expect("tier map poisoned").clear();
    }

    async fn loaded_(&self) -> bool {
        self.loaded().await
            || self
                .tiers
                .lock()
                .expect("tier map poisoned")
                .values()
                .any(Option::is_some)
    }

    async fn reload_(&self) -> anyhow::Result<()> {
        self.reload().await?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl Inpainter for LamaLargeInpainter {
    async fn inpaint(
        &self,
        image: &interface_image::RawImage,
        mask: interface_image::Mask,
        options: InpainterOptions,
        img_processor: &Arc<dyn ImageOp + Send + Sync>,
    ) -> anyhow::Result<interface_image::RawImage> {
        let ho = image.height;
        let wo = image.width;
        let (image, mask) =
            lama_resize_image(image.view(), mask, options.inpainting_size, img_processor)?;
        let mut image = image.to_owned();
        let h = image.height;
        let w = image.width;
        image = interface_inpainter::remove_mask_area(image, &mask);
        let (image, mask, w16, h16) = lama_add_border(image, mask, img_processor);

        let tier = self.tier_session(w16, h16).await;
        let (new_w, new_h) = tier
            .as_ref()
            .map_or((w16, h16), |(t, _)| (t.width, t.height));
        let dynamic;
        let session: &Mutex<Session> = match &tier {
            Some((_, session)) => session.as_ref(),
            None => {
                dynamic = self.load().await?;
                &*dynamic
            }
        };

        // Synchronous `Session::run` on purpose: the `ort-parallel` pool (ORT `RunAsync`) ran the
        // first two inferences of every session at ~67s instead of ~12s on a 1488x2048 image and
        // stayed ~15% slower afterwards.
        let img = spawn_blocking!(|| {
            // The position encoding is computed before the tier padding: the padding is unmasked, where
            // rel_pos and direct are zero anyway, and the 256x256 downsample keeps the page's aspect.
            let (rel_pos, direct) = mpe::load_masked_position_encoding(mask.view(), img_processor)?;
            let (image, mask) = lama_pad_canvas(image, mask, new_w, new_h, img_processor);
            let mask = mask
                .as_nd()?
                .mapv(|v| if v >= 127 { 1.0f32 } else { 0.0f32 })
                .insert_axis(Axis(0))
                .insert_axis(Axis(0));
            let image = image
                .as_ndarray()
                .unwrap()
                .permuted_axes((2, 0, 1))
                .mapv(|v| v as f32 / 255.0)
                .insert_axis(Axis(0));
            let image = Tensor::from_array(image)?;
            let mask = Tensor::from_array(mask)?;
            let (rel_pos, direct) = if tier.is_some() {
                let canvas = (new_h as usize, new_w as usize);
                let mut rel_pos_c = Array2::<i64>::zeros(canvas);
                rel_pos_c
                    .slice_mut(s![..h16 as usize, ..w16 as usize])
                    .assign(&rel_pos);
                // The static exports take `direct` as float32 (the export script folds the model's Cast
                // into the input because ORT 1.22's CoreML EP has no builder for it).
                let mut direct_c = Array3::<f32>::zeros((canvas.0, canvas.1, 4));
                direct_c
                    .slice_mut(s![..h16 as usize, ..w16 as usize, ..])
                    .zip_mut_with(&direct, |d, &v| *d = v as f32);
                (
                    Tensor::from_array(rel_pos_c.insert_axis(Axis(0)))?,
                    Tensor::from_array(direct_c.insert_axis(Axis(0)))?.into_dyn(),
                )
            } else {
                (
                    Tensor::from_array(rel_pos.insert_axis(Axis(0)))?,
                    Tensor::from_array(direct.insert_axis(Axis(0)))?.into_dyn(),
                )
            };

            let mut session = session.lock().expect("session mutex poisoned");
            let out = session.run(
                inputs! {"image"=> image, "mask"=> mask, "rel_pos" => rel_pos, "direct" => direct},
            )?;
            let out: ArrayView4<f32> = out[0].try_extract_array()?.into_dimensionality()?;
            let img_inpainted = out
                .remove_axis(Axis(0))
                .permuted_axes((1, 2, 0))
                .mapv(|v| (v * 255.0) as u8);
            let mut img_inpainted = RawImageCow::from(img_inpainted.view());
            if new_h != h || new_w != w {
                img_inpainted =
                    RawImageCow::Owned(img_processor.remove_border(img_inpainted.view(), w, h));
            }
            if h != ho || w != wo {
                img_inpainted = RawImageCow::Owned(img_processor.resize(
                    img_inpainted.view(),
                    wo,
                    ho,
                    interface_image::Interpolation::Bicubic,
                )?);
            }
            Ok::<_, anyhow::Error>(img_inpainted.to_owned())
        })??;

        Ok(img)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use base_util::onnx::all_providers;
    use interface_image::{CpuImageProcessor, Mask, RawImage};
    use ndarray::Array2;

    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn test_inpaint() {
        let img = RawImage::new("./imgs/232265329-6a560438-e887-4f7f-b6a1-a61b8648f781.png")
            .expect("Failed to load image");
        let img_processor =
            Arc::new(CpuImageProcessor::default()) as Arc<dyn ImageOp + Send + Sync>;
        let mask: Array2<u8> = ndarray_npy::read_npy("../lama_large/mask.npy").unwrap();
        let mask = Mask::from(mask);
        let inp = LamaLargeInpainter::new(Arc::new(all_providers()));
        let v = inp
            .inpaint(&img, mask, Default::default(), &img_processor)
            .await
            .unwrap();
        assert_eq!((v.width, v.height), (img.width, img.height));
        v.to_image().unwrap().save("inpainted.png").unwrap()
    }
}
