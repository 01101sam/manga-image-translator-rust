use std::{
    fmt::Display,
    sync::{Arc, Mutex},
};

use base_util::onnx::{new_session, Providers};
use interface_image::{
    combine_patches_m, generate_patches, DimType, ImageOp, RawImage, RawImageCow,
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

pub struct Waifu2xUpscaler {
    model: ModelWrap<Mutex<Session>>,
    model_kind: Waifu2xModels,
    max_batch_size: usize,
    providers: Arc<Vec<Providers>>,
}

impl Display for Waifu2xModels {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Waifu2xModels::CuNetArt { noise } => format!(
                    "cunet-art-2x{}",
                    noise.map(|v| format!("-noise{v}")).unwrap_or_default()
                ),
                Waifu2xModels::SwinUnetArt { x4, noise } => format!(
                    "swin_unet-art-{}x{}",
                    if *x4 { "4" } else { "2" },
                    noise.map(|v| format!("-noise{v}")).unwrap_or_default()
                ),
                Waifu2xModels::SwinUnetArtScan { x4, noise } => format!(
                    "swin_unet-art_scan-{}x{}",
                    if *x4 { "4" } else { "2" },
                    noise.map(|v| format!("-noise{v}")).unwrap_or_default()
                ),
                Waifu2xModels::SwinUnetArtPhoto { x4, noise } => format!(
                    "swin_unet-photo-{}x{}",
                    if *x4 { "4" } else { "2" },
                    noise.map(|v| format!("-noise{v}")).unwrap_or_default()
                ),
            }
        )
    }
}

pub enum Waifu2xModels {
    CuNetArt { noise: Option<u8> },
    SwinUnetArt { x4: bool, noise: Option<u8> },
    SwinUnetArtScan { x4: bool, noise: Option<u8> },
    SwinUnetArtPhoto { x4: bool, noise: Option<u8> },
}

impl Waifu2xModels {
    fn scale(&self) -> usize {
        match self {
            Waifu2xModels::CuNetArt { .. } => 2,
            Waifu2xModels::SwinUnetArt { x4, .. } => {
                if *x4 {
                    4
                } else {
                    2
                }
            }
            Waifu2xModels::SwinUnetArtScan { x4, .. } => {
                if *x4 {
                    4
                } else {
                    2
                }
            }
            Waifu2xModels::SwinUnetArtPhoto { x4, .. } => {
                if *x4 {
                    4
                } else {
                    2
                }
            }
        }
    }
}

impl Waifu2xUpscaler {
    pub fn new(
        model: Waifu2xModels,
        max_batch_size: usize,
        providers: Arc<Vec<Providers>>,
    ) -> Self {
        Self {
            model: Default::default(),
            model_kind: model,
            max_batch_size,
            providers,
        }
    }
}

#[async_trait::async_trait]
impl ModelLoad for Waifu2xUpscaler {
    impl_model_load_helpers!(model, Mutex<Session>);

    async fn reload(&self) -> anyhow::Result<ModelRead<'_, Self::T>> {
        let model = self.model_kind.to_string();
        let path = self
            .download_model(&model, &format!("{model}.onnx"))
            .await?;
        // CoreML loses or breaks on every variant (M3 Max, sync run): cunet runs 512ms vs 440ms
        // on the CPU EP at 371x512 and 1.95s vs 1.55s at 743x1024 under NeuralNetwork, and the
        // MLProgram format fails to compile (error -14); swin_unet shatters into 90+ partitions
        // and fails at predict time under NeuralNetwork (error -1), MLProgram again -14.
        let providers: Vec<_> = self
            .providers
            .iter()
            .filter(|p| !matches!(p, Providers::CoreML))
            .cloned()
            .collect();
        let session = new_session(&providers)?.commit_from_file(&path)?;
        *self.model.write().await = Some(Mutex::new(session));
        Ok(self.get_model().await.expect("Set model before"))
    }
}

impl Model for Waifu2xUpscaler {
    impl_model_helpers!("upscaler", "waifu2x", model);
    fn models(&self) -> std::collections::HashMap<&'static str, interface_model::ModelSource> {
        hashmap! {
            "cunet-art-2x-noise0" => ModelSource { url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/waifu2x-20250502/cunet-art-2x-noise0.onnx", hash: "###" },
            "cunet-art-2x-noise1" => ModelSource { url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/waifu2x-20250502/cunet-art-2x-noise1.onnx", hash: "###" },
            "cunet-art-2x-noise2" => ModelSource { url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/waifu2x-20250502/cunet-art-2x-noise2.onnx", hash: "###" },
            "cunet-art-2x-noise3" => ModelSource { url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/waifu2x-20250502/cunet-art-2x-noise3.onnx", hash: "###" },
            "cunet-art-2x" => ModelSource { url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/waifu2x-20250502/cunet-art-2x.onnx", hash: "###" },
            "swin_unet-art-2x-noise0" => ModelSource { url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/waifu2x-20250502/swin_unet-art-2x-noise0.onnx", hash: "###" },
            "swin_unet-art-2x-noise1" => ModelSource { url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/waifu2x-20250502/swin_unet-art-2x-noise1.onnx", hash: "###" },
            "swin_unet-art-2x-noise2" => ModelSource { url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/waifu2x-20250502/swin_unet-art-2x-noise2.onnx", hash: "###" },
            "swin_unet-art-2x-noise3" => ModelSource { url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/waifu2x-20250502/swin_unet-art-2x-noise3.onnx", hash: "###" },
            "swin_unet-art-2x" => ModelSource { url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/waifu2x-20250502/swin_unet-art-2x.onnx", hash: "###" },
            "swin_unet-art-4x-noise0" => ModelSource { url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/waifu2x-20250502/swin_unet-art-4x-noise0.onnx", hash: "###" },
            "swin_unet-art-4x-noise1" => ModelSource { url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/waifu2x-20250502/swin_unet-art-4x-noise1.onnx", hash: "###" },
            "swin_unet-art-4x-noise2" => ModelSource { url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/waifu2x-20250502/swin_unet-art-4x-noise2.onnx", hash: "###" },
            "swin_unet-art-4x-noise3" => ModelSource { url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/waifu2x-20250502/swin_unet-art-4x-noise3.onnx", hash: "###" },
            "swin_unet-art-4x" => ModelSource { url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/waifu2x-20250502/swin_unet-art-4x.onnx", hash: "###" },
            "swin_unet-art_scan-2x-noise0" => ModelSource { url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/waifu2x-20250502/swin_unet-art_scan-2x-noise0.onnx", hash: "###" },
            "swin_unet-art_scan-2x-noise1" => ModelSource { url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/waifu2x-20250502/swin_unet-art_scan-2x-noise1.onnx", hash: "###" },
            "swin_unet-art_scan-2x-noise2" => ModelSource { url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/waifu2x-20250502/swin_unet-art_scan-2x-noise2.onnx", hash: "###" },
            "swin_unet-art_scan-2x-noise3" => ModelSource { url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/waifu2x-20250502/swin_unet-art_scan-2x-noise3.onnx", hash: "###" },
            "swin_unet-art_scan-2x" => ModelSource { url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/waifu2x-20250502/swin_unet-art_scan-2x.onnx", hash: "###" },
            "swin_unet-art_scan-4x-noise0" => ModelSource { url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/waifu2x-20250502/swin_unet-art_scan-4x-noise0.onnx", hash: "###" },
            "swin_unet-art_scan-4x-noise1" => ModelSource { url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/waifu2x-20250502/swin_unet-art_scan-4x-noise1.onnx", hash: "###" },
            "swin_unet-art_scan-4x-noise2" => ModelSource { url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/waifu2x-20250502/swin_unet-art_scan-4x-noise2.onnx", hash: "###" },
            "swin_unet-art_scan-4x-noise3" => ModelSource { url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/waifu2x-20250502/swin_unet-art_scan-4x-noise3.onnx", hash: "###" },
            "swin_unet-art_scan-4x" => ModelSource { url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/waifu2x-20250502/swin_unet-art_scan-4x.onnx", hash: "###" },
            "swin_unet-photo-2x-noise0" => ModelSource { url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/waifu2x-20250502/swin_unet-photo-2x-noise0.onnx", hash: "###" },
            "swin_unet-photo-2x-noise1" => ModelSource { url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/waifu2x-20250502/swin_unet-photo-2x-noise1.onnx", hash: "###" },
            "swin_unet-photo-2x-noise2" => ModelSource { url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/waifu2x-20250502/swin_unet-photo-2x-noise2.onnx", hash: "###" },
            "swin_unet-photo-2x-noise3" => ModelSource { url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/waifu2x-20250502/swin_unet-photo-2x-noise3.onnx", hash: "###" },
            "swin_unet-photo-2x" => ModelSource { url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/waifu2x-20250502/swin_unet-photo-2x.onnx", hash: "###" },
            "swin_unet-photo-4x-noise0" => ModelSource { url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/waifu2x-20250502/swin_unet-photo-4x-noise0.onnx", hash: "###" },
            "swin_unet-photo-4x-noise1" => ModelSource { url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/waifu2x-20250502/swin_unet-photo-4x-noise1.onnx", hash: "###" },
            "swin_unet-photo-4x-noise2" => ModelSource { url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/waifu2x-20250502/swin_unet-photo-4x-noise2.onnx", hash: "###" },
            "swin_unet-photo-4x-noise3" => ModelSource { url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/waifu2x-20250502/swin_unet-photo-4x-noise3.onnx", hash: "###" },
            "swin_unet-photo-4x" => ModelSource { url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/waifu2x-20250502/swin_unet-photo-4x.onnx", hash: "###" },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use base_util::onnx::all_providers;
    use interface_image::{CpuImageProcessor, DimType, ImageOp, RawImage};
    use interface_upscaler::Upscaler as _;

    use crate::{Waifu2xModels, Waifu2xUpscaler};

    #[tokio::test(flavor = "multi_thread")]
    async fn test_upscaler() {
        let upscaler = Waifu2xUpscaler::new(
            Waifu2xModels::CuNetArt { noise: Some(3) },
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

        assert_eq!(upscaled.width, w * upscaler.model_kind.scale() as DimType);
        assert_eq!(upscaled.height, w * upscaler.model_kind.scale() as DimType);
        upscaled.to_image().unwrap().save("upscaled.png").unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_upscaler_patches() {
        let upscaler = Waifu2xUpscaler::new(
            Waifu2xModels::CuNetArt { noise: Some(3) },
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
                0,
                &(img_processor as Arc<dyn ImageOp + Send + Sync>),
            )
            .await
            .expect("Failed to upscale image");
        assert_eq!(upscaled.width, w * upscaler.model_kind.scale() as DimType);
        assert_eq!(upscaled.height, w * upscaler.model_kind.scale() as DimType);
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
    image: &RawImage,
    patch_size: Option<usize>,
    padding: usize,
    max_batch_size: usize,
    img_processor: &Arc<dyn ImageOp + Send + Sync>,
) -> anyhow::Result<Vec<Array4<f32>>> {
    let pad_x = (8 - image.width % 8) % 8;
    let pad_y = (8 - image.height % 8) % 8;
    let w = image.width;
    let h = image.height;
    let imgs = match patch_size {
        Some(v) => generate_patches(image.view(), v, 18 + padding)
            .into_iter()
            .map(RawImageCow::Owned)
            .collect::<Vec<_>>(),
        None => {
            let mut img = RawImageCow::Borrowed(image.view());
            if let RawImageCow::Owned(v) =
                img_processor.add_border_wh(img.view(), w + pad_x, h + pad_y)
            {
                img = RawImageCow::Owned(v);
            }
            if let RawImageCow::Owned(v) = img_processor.add_border_center_wh(
                img.view(),
                w + pad_x + 18 * 2,
                h + pad_y + 18 * 2,
            ) {
                img = RawImageCow::Owned(v);
            }
            vec![img]
        }
    };

    let patches = imgs
        .into_iter()
        .map(|v| {
            v.view()
                .as_ndarray()
                .unwrap()
                .mapv(|v| v as f32 / 255.0)
                .permuted_axes((2, 0, 1))
        })
        .collect::<Vec<_>>();
    Ok(join_batches(
        patches.iter().map(|v| v.view()).collect(),
        max_batch_size,
    )?)
}

/// Synchronous `Session::run` on purpose: the `ort-parallel` pool (ORT `RunAsync`) ran the first
/// two inferences of every session at 3.7-5s instead of 0.7s (cunet, 371x512) and stayed slower.
fn process(model: &Mutex<Session>, batches: Vec<Array4<f32>>) -> anyhow::Result<Vec<Array3<u8>>> {
    let mut processed_patches = vec![];
    let mut session = model.lock().expect("session mutex poisoned");
    for batch in batches {
        let out = session.run(inputs! {"x"=>Tensor::from_array(batch)?})?;
        let img: ArrayViewD<f32> = out[0].try_extract_array()?;
        let img: ArrayView4<f32> = img.into_dimensionality()?;
        for img in img.outer_iter() {
            let img = img.permuted_axes((1, 2, 0)).mapv(|v| (v * 255.0) as u8);
            processed_patches.push(img);
        }
    }
    Ok(processed_patches)
}

#[async_trait::async_trait]
impl Upscaler for Waifu2xUpscaler {
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

        let model = self.load().await?;
        let out = spawn_blocking!(|| {
            let batches = pre_process(image, patch_size, padding, max_batch_size, img_processor)?;
            let mut patches = process(&model, batches)?
                .into_iter()
                .map(RawImage::from)
                .collect::<Vec<_>>();
            let ps = patches[0].width;
            Ok::<_, anyhow::Error>(match patch_size {
                Some(_) => combine_patches_m(
                    patches,
                    w * self.model_kind.scale() as DimType,
                    h * self.model_kind.scale() as DimType,
                    ps as usize,
                    padding * self.model_kind.scale(),
                ),
                None => img_processor.remove_border(
                    patches.remove(0).view(),
                    w * self.model_kind.scale() as DimType,
                    h * self.model_kind.scale() as DimType,
                ),
            })
        })??;

        Ok(out)
    }
}
