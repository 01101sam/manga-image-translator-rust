use std::{
    fmt::Display,
    sync::{Arc, Mutex},
};

use base_util::onnx::{new_session, Providers};
use half::f16;
use interface_image::{
    combine_patches, generate_patches, DimType, ImageOp, RawImage, RawImageCow, RawImageView,
};
use interface_model::{
    impl_model_helpers, impl_model_load_helpers, Model, ModelLoad, ModelRead, ModelSource,
    ModelWrap,
};
use interface_upscaler::Upscaler;
use maplit::hashmap;
use ndarray::{stack, Array3, Array4, ArrayView, ArrayView4, ArrayViewD, Axis, Dimension};
use ort::{inputs, session::Session, value::Tensor};
use util::spawn_blocking;

pub struct EsrGan {
    model: ModelWrap<Mutex<Session>>,
    model_kind: EsrGanModel,
    max_batch_size: usize,
    providers: Arc<Vec<Providers>>,
}

pub enum EsrGanModel {
    X2Plus { f32: bool },
    X4Plus { f32: bool },
    X4PlusAnime6B { f32: bool },
}

impl EsrGanModel {
    pub fn half(&self) -> bool {
        match self {
            EsrGanModel::X2Plus { f32 } => !*f32,
            EsrGanModel::X4Plus { f32 } => !*f32,
            EsrGanModel::X4PlusAnime6B { f32 } => !*f32,
        }
    }
    pub fn zoom(&self) -> usize {
        match self {
            EsrGanModel::X2Plus { .. } => 2,
            EsrGanModel::X4Plus { .. } => 4,
            EsrGanModel::X4PlusAnime6B { .. } => 4,
        }
    }
}

impl Display for EsrGanModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                EsrGanModel::X2Plus { f32 } =>
                    format!("x2plus-f{}", if *f32 { "32" } else { "16" }),
                EsrGanModel::X4Plus { f32 } =>
                    format!("x4plus-f{}", if *f32 { "32" } else { "16" }),
                EsrGanModel::X4PlusAnime6B { f32 } =>
                    format!("x4plus_anime_6B-f{}", if *f32 { "32" } else { "16" }),
            }
        )
    }
}
impl EsrGan {
    pub fn new(model: EsrGanModel, max_batch_size: usize, providers: Arc<Vec<Providers>>) -> Self {
        Self {
            model: Default::default(),
            model_kind: model,
            max_batch_size,
            providers,
        }
    }
}

#[async_trait::async_trait]
impl ModelLoad for EsrGan {
    impl_model_load_helpers!(model, Mutex<Session>);

    async fn reload(&self) -> anyhow::Result<ModelRead<'_, Self::T>> {
        let model = self.model_kind.to_string();
        let path = self
            .download_model(&model, &format!("{model}.onnx"))
            .await?;
        // CoreML NeuralNetwork runs the f32 exports whole on the GPU: x4plus 1.0s vs 9.8s on the
        // CPU EP per 371x512 upscale (M3 Max), output within 8/255 of the CPU EP. MLProgram is
        // 10% slower with a longer first load. The f16 exports it does not take at all (either
        // format falls back to the CPU EP, ~9s), so on CoreML request f32 variants.
        let session = new_session(&self.providers)?.commit_from_file(&path)?;
        *self.model.write().await = Some(Mutex::new(session));

        Ok(self.get_model().await.expect("Set model before"))
    }
}

#[async_trait::async_trait]
impl Model for EsrGan {
    impl_model_helpers!("upscaler", "waifu2x", model);
    fn models(&self) -> std::collections::HashMap<&'static str, interface_model::ModelSource> {
        hashmap! {
            "x2plus-f16" => ModelSource { url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/realesrgan/x2plus-f16.onnx", hash: "###" },
            "x2plus-f32" => ModelSource { url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/realesrgan/x2plus-f32.onnx", hash: "###" },
            "x4plus-f16" => ModelSource { url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/realesrgan/x4plus-f16.onnx", hash: "###" },
            "x4plus-f32" => ModelSource { url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/realesrgan/x4plus-f32.onnx", hash: "###" },
            "x4plus_anime_6B-f16" => ModelSource { url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/realesrgan/x4plus_anime_6B-f16.onnx", hash: "###" },
            "x4plus_anime_6B-f32" => ModelSource { url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/realesrgan/x4plus_anime_6B-f32.onnx", hash: "###" }
        }
    }
}

pub fn join_batches<A: Clone, Dim: Dimension>(
    patches: Vec<ArrayView<'_, A, Dim>>,
    max: usize,
) -> Result<
    Vec<ndarray::ArrayBase<ndarray::OwnedRepr<A>, <Dim as Dimension>::Larger>>,
    ndarray::ShapeError,
> {
    let v = patches
        .chunks(max)
        .map(|v| stack(Axis(0), v))
        .collect::<Result<Vec<_>, ndarray::ShapeError>>()?;
    Ok(v)
}

fn pre_process(
    image: RawImageView,
    patch_size: Option<usize>,
    padding: usize,
    max_batch_size: usize,
    img_processor: &Arc<dyn ImageOp + Send + Sync>,
) -> anyhow::Result<Vec<Array4<f16>>> {
    let pad_x = (8 - image.width % 8) % 8;
    let pad_y = (8 - image.height % 8) % 8;
    let w = image.width;
    let h = image.height;
    let imgs = match patch_size {
        Some(v) => generate_patches(image, v, padding)
            .into_iter()
            .map(|v| RawImageCow::Owned(v))
            .collect::<Vec<_>>(),
        None => {
            let mut inp = RawImageCow::Borrowed(image);
            if let RawImageCow::Owned(v) =
                img_processor.add_border_wh(inp.view(), w + pad_x, h + pad_y)
            {
                inp = RawImageCow::Owned(v);
            }
            if let RawImageCow::Owned(v) =
                img_processor.add_border_center_wh(inp.view(), w + pad_x, h + pad_y)
            {
                inp = RawImageCow::Owned(v);
            }
            vec![inp]
        }
    };

    let patches = imgs
        .into_iter()
        .map(|v| {
            v.view()
                .as_ndarray()
                .unwrap()
                .mapv(|v| f16::from_f32(v as f32 / 255.0))
                .permuted_axes((2, 0, 1))
        })
        .collect::<Vec<_>>();
    Ok(join_batches(
        patches.iter().map(|v| v.view()).collect(),
        max_batch_size,
    )?)
}

/// Synchronous `Session::run` on purpose: the `ort-parallel` pool (ORT `RunAsync`) ran the first
/// two inferences of every session at ~85s instead of ~14s (x4plus-f16, 371x512).
fn process(
    model: &Mutex<Session>,
    batches: Vec<Array4<f16>>,
    half: bool,
) -> anyhow::Result<Vec<Array3<u8>>> {
    let mut processed_patches = vec![];
    let mut session = model.lock().expect("session mutex poisoned");
    for batch in batches {
        if half {
            let out = session.run(inputs! {"input"=>Tensor::from_array(batch)?})?;
            let img: ArrayViewD<f16> = out[0].try_extract_array()?;
            let img: ArrayView4<f16> = img.into_dimensionality()?;
            for img in img.outer_iter() {
                let img = img
                    .permuted_axes((1, 2, 0))
                    .mapv(|v| (v.to_f32() * 255.0) as u8);
                processed_patches.push(img);
            }
        } else {
            let batch = batch.mapv(|v| v.to_f32());
            let out = session.run(inputs! {"input"=>Tensor::from_array(batch)?})?;
            let img: ArrayViewD<f32> = out[0].try_extract_array()?;
            let img: ArrayView4<f32> = img.into_dimensionality()?;
            for img in img.outer_iter() {
                let img = img.permuted_axes((1, 2, 0)).mapv(|v| (v * 255.0) as u8);
                processed_patches.push(img);
            }
        };
    }
    Ok(processed_patches)
}

#[async_trait::async_trait]
impl Upscaler for EsrGan {
    async fn upscale(
        &self,
        image: &RawImage,
        patch_size: Option<usize>,
        padding: usize,
        img_processor: &Arc<dyn ImageOp + Send + Sync>,
    ) -> anyhow::Result<RawImage> {
        let max_batch_size = self.max_batch_size;
        let w = image.width;
        let h = image.height;
        let half = self.model_kind.half();

        let model = self.load().await?;
        let out = spawn_blocking!(|| {
            let batches =
                pre_process(image.view(), patch_size, padding, max_batch_size, img_processor)?;
            let mut patches = process(&model, batches, half)?
                .into_iter()
                .map(RawImage::from)
                .collect::<Vec<_>>();
            let ps = patches[0].width;
            Ok::<_, anyhow::Error>(match patch_size {
                Some(_) => combine_patches(
                    patches,
                    w * self.model_kind.zoom() as DimType,
                    h * self.model_kind.zoom() as DimType,
                    ps as usize,
                    padding * self.model_kind.zoom(),
                ),
                None => img_processor.remove_border(
                    patches.remove(0).view(),
                    w * self.model_kind.zoom() as DimType,
                    h * self.model_kind.zoom() as DimType,
                ),
            })
        })??;

        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use base_util::onnx::all_providers;
    use interface_image::{CpuImageProcessor, DimType, ImageOp, RawImage};
    use interface_upscaler::Upscaler as _;

    use crate::{EsrGan, EsrGanModel};

    #[tokio::test(flavor = "multi_thread")]
    async fn test_upscaler() {
        let upscaler = EsrGan::new(
            EsrGanModel::X2Plus { f32: true },
            5,
            Arc::new(all_providers()),
        );
        let image = RawImage::url(
            "https://github.com/xinntao/Real-ESRGAN/blob/master/inputs/0014.jpg?raw=true",
        )
        .expect("Failed to load image");
        let w = image.width;
        let img_processor = Arc::new(CpuImageProcessor::default());
        let upscaled = upscaler
            .upscale(
                &image,
                None,
                0,
                &(img_processor as Arc<dyn ImageOp + Send + Sync>),
            )
            .await
            .expect("Failed to upscale image");
        assert_eq!(upscaled.width, w * upscaler.model_kind.zoom() as DimType);
        assert_eq!(upscaled.height, w * upscaler.model_kind.zoom() as DimType);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_upscaler_patches() {
        let upscaler = EsrGan::new(
            EsrGanModel::X4PlusAnime6B { f32: false },
            5,
            Arc::new(all_providers()),
        );
        let image = RawImage::url(
            "https://github.com/xinntao/Real-ESRGAN/blob/master/inputs/0014.jpg?raw=true",
        )
        .expect("Failed to load image");
        let w = image.width;
        let img_processor = Arc::new(CpuImageProcessor::default());
        let upscaled = upscaler
            .upscale(
                &image,
                Some(100),
                10,
                &(img_processor as Arc<dyn ImageOp + Send + Sync>),
            )
            .await
            .expect("Failed to upscale image");
        assert_eq!(upscaled.width, w * upscaler.model_kind.zoom() as DimType);
        assert_eq!(upscaled.height, w * upscaler.model_kind.zoom() as DimType);
    }
}
