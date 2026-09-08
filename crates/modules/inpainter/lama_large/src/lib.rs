use std::sync::{Arc, Mutex};

use base_util::onnx::{new_session, Providers};
use interface_image::{ImageOp, RawImageCow};
use interface_inpainter::{Inpainter, InpainterOptions};
use interface_model::{
    impl_model_helpers, impl_model_load_helpers, Model, ModelLoad, ModelRead, ModelSource,
    ModelWrap,
};
use maplit::hashmap;
use ndarray::{ArrayView4, Axis};
use ort::{inputs, session::Session, value::Tensor};
use util::{
    lama::{lama_add_border, lama_resize_image},
    spawn_blocking,
};

pub struct LamaLargeInpainter {
    model: ModelWrap<Mutex<Session>>,
    providers: Arc<Vec<Providers>>,
}

impl LamaLargeInpainter {
    pub fn new(providers: Arc<Vec<Providers>>) -> Self {
        Self {
            model: Default::default(),
            providers,
        }
    }
}

#[async_trait::async_trait]
impl ModelLoad for LamaLargeInpainter {
    impl_model_load_helpers!(model, Mutex<Session>);
    async fn reload(&self) -> anyhow::Result<ModelRead<'_, Self::T>> {
        let p = self.download_model("model", "model.onnx").await?;
        // Same FFC spectral path as lama_mpe: CoreML cannot host it and shatters the graph into
        // hundreds of partitions. On a 1487x2048 page (M3 Max) that ran 47s, 130s, 302s on
        // consecutive inferences (memory growing to ~120GB) against a steady 15.7s on the CPU EP.
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

impl Model for LamaLargeInpainter {
    impl_model_helpers!("inpainter", "lama_large", model);

    fn models(&self) -> std::collections::HashMap<&'static str, interface_model::ModelSource> {
        hashmap! {"model" => ModelSource{ url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/lama_large_512px/model.onnx", hash: "###" }}
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
        let session = self.load().await?;
        // Synchronous `Session::run` on purpose: the `ort-parallel` pool (ORT `RunAsync`) ran the
        // first two inferences of every session at ~113s instead of ~18s on a 1487x2048 page and
        // stayed slower afterwards.
        let img = spawn_blocking!(|| {
            let (image, mask) =
                lama_resize_image(image.view(), mask, options.inpainting_size, img_processor)?;
            let mut image = image.to_owned();
            let h = image.height;
            let w = image.width;
            image = interface_inpainter::remove_mask_area(image, &mask);

            let (image, mask, new_w, new_h) = lama_add_border(image, mask, img_processor);
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

            let mut session = session.lock().expect("session mutex poisoned");
            let out = session.run(inputs! {"image"=> image, "mask"=> mask})?;
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

    use interface_image::{CpuImageProcessor, Mask, RawImage};
    use ndarray::Array2;

    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn test_inpaint() {
        let img = RawImage::new("./imgs/232265329-6a560438-e887-4f7f-b6a1-a61b8648f781.png")
            .expect("Failed to load image");
        let img_processor =
            Arc::new(CpuImageProcessor::default()) as Arc<dyn ImageOp + Send + Sync>;
        let mask: Array2<u8> = ndarray_npy::read_npy("mask.npy").unwrap();
        let mask = Mask::from(mask);
        let inp = LamaLargeInpainter::new(Default::default());
        let v = inp
            .inpaint(&img, mask, Default::default(), &img_processor)
            .await
            .unwrap();
        assert_eq!((v.width, v.height), (img.width, img.height));
        v.to_image().unwrap().save("inpainted.png").unwrap()
    }
}
