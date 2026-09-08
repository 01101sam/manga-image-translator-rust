mod refine_mask;

use std::{
    ops::Deref,
    sync::{Arc, Mutex},
};

use anyhow::anyhow;
use base_util::onnx::{new_session_mlprogram, Providers};

use interface_detector::{textlines::Quadrilateral, DefaultOptions, Detector};
use interface_image::{ImageOp, Interpolation, Mask, RawImageCow, RawImageView};
use interface_model::{
    impl_model_helpers, impl_model_load_helpers, Model, ModelLoad, ModelRead, ModelSource,
    ModelWrap,
};
use maplit::hashmap;
use ndarray::{s, stack, Array2, Array4, ArrayView4, ArrayViewD, Axis};
use ort::{session::Session, value::Tensor};
use util::{
    dbnet::SegDetectorRepresenter,
    det_arrange::{det_rearrange_forward, det_unarrange, shoud_rearrange},
};

pub struct CtdDetector {
    providers: Arc<Vec<Providers>>,
    model: ModelWrap<Mutex<Session>>,
}

impl CtdDetector {
    ///convnext: Different model architecture, but based on dbnet
    pub fn new(providers: Arc<Vec<Providers>>) -> Self {
        CtdDetector {
            providers,
            model: Default::default(),
        }
    }
}

#[async_trait::async_trait]
impl ModelLoad for CtdDetector {
    impl_model_load_helpers!(model, Mutex<Session>);

    async fn reload(&self) -> anyhow::Result<ModelRead<'_, Self::T>> {
        let p = self.download_model("model", "model.onnx").await?;
        let session = new_session_mlprogram(&self.providers)?.commit_from_file(&p)?;
        *self.model.write().await = Some(Mutex::new(session));
        Ok(self.get_model().await.expect("Model was set before"))
    }
}

impl Model for CtdDetector {
    impl_model_helpers!("detector", "ctd", model);

    fn models(&self) -> std::collections::HashMap<&'static str, ModelSource> {
        hashmap! {
            "model" => ModelSource { url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/ctd-v1.0.0/model.onnx", hash: "c921d44fea30913a1689dcb4d28faef664dfd0c9f895146d27342e52b823ec0c" }
        }
    }
}

#[async_trait::async_trait]
impl Detector for CtdDetector {
    async fn infer(
        &self,
        img: RawImageCow<'_>,
        _: DefaultOptions,
        img_processor: &Arc<dyn ImageOp + Send + Sync>,
    ) -> anyhow::Result<(Vec<Quadrilateral>, Mask)> {
        let img_ = img.view();
        let (im_w, im_h) = (img_.width, img_.height);
        let session = self.load().await?;
        let session = session.deref();
        let (lines_map, mask) = match shoud_rearrange(img_, 1024) {
            true => {
                let (batch_list, rv) = det_rearrange_forward(img.view(), 1024, 4, img_processor)?;
                let out = batch_list
                    .iter()
                    .map(|batch| det_batch_forward_ctd(session, batch.view()))
                    .collect::<anyhow::Result<Vec<_>>>()?;
                det_unarrange(out, rv)?
            }
            false => {
                let (img_in, _, dw, dh) =
                    preprocess_img(img.view(), img_processor, (1024, 1024), true, false)?;
                let tensor = Tensor::from_array(img_in)?;
                let mut session = session.lock().expect("session mutex poisoned");
                let outputs = session.run(ort::inputs!["input" => tensor])?;

                let mask: ArrayViewD<f32> = outputs["mask"].try_extract_array()?;
                let lines_map: ArrayViewD<f32> = outputs["lines"].try_extract_array()?;
                let mut lines_map = lines_map.into_dimensionality::<ndarray::Ix4>()?.to_owned();
                let mut mask = mask.into_dimensionality::<ndarray::Ix4>()?.to_owned();
                let mask_shape = mask.shape().to_vec();
                let lines_shape = lines_map.shape().to_vec();
                let dh = dh as usize;
                let dw = dw as usize;
                mask = mask.slice_move(s![.., .., 0..mask_shape[2] - dh, 0..mask_shape[3] - dw]);

                lines_map = lines_map.slice_move(s![
                    ..,
                    ..,
                    0..lines_shape[2] - dh,
                    0..lines_shape[3] - dw
                ]);
                (lines_map, mask)
            }
        };

        let mask: Array2<f32> = mask
            .index_axis(ndarray::Axis(0), 0)
            .index_axis(ndarray::Axis(0), 0)
            .to_owned();

        let mask = Mask::from(mask.mapv(|v| f32::clamp(v * 255.0, 0.0, 255.0) as u8));
        let (lines, scores) =
            SegDetectorRepresenter::default().call(lines_map, false, im_w as u16, im_h as u16)?;
        let box_thresh = 0.6;
        let scores = scores
            .into_iter()
            .flatten()
            .next()
            .ok_or(anyhow!("no scores"))?;
        let lines = lines
            .into_iter()
            .flatten()
            .next()
            .ok_or(anyhow!("no lines"))?;
        let qu = lines
            .outer_iter()
            .zip(scores)
            .filter(|(_, score)| *score > box_thresh)
            .map(|(points, score)| {
                Quadrilateral::new(
                    points
                        .outer_iter()
                        .map(|v| (v[0], v[1]))
                        .collect::<Vec<(i64, i64)>>(),
                    score,
                )
            })
            .collect::<Vec<_>>();
        let mask = img_processor.resize_mask(
            mask.view(),
            im_w as usize,
            im_h as usize,
            Interpolation::Bilinear,
        )?;

        let mask_refined = refine_mask::refine_mask(&img, mask, &qu, false)?;

        Ok((qu, mask_refined))
    }
}
/// Runs the session synchronously on the calling thread; see `dbnet` for why `RunAsync` is avoided.
fn det_batch_forward_ctd(
    session: &Mutex<Session>,
    batch: ArrayView4<u8>,
) -> anyhow::Result<(Array4<f32>, Array4<f32>)> {
    let batch = batch.mapv(|v| v as f32 / 255.).permuted_axes([0, 3, 1, 2]);
    let tensor = Tensor::from_array(batch)?;
    let mut session = session.lock().expect("session mutex poisoned");
    let outputs = session.run(ort::inputs!["input" => tensor])?;

    let mask: ArrayViewD<f32> = outputs["mask"].try_extract_array()?;
    let lines: ArrayViewD<f32> = outputs["lines"].try_extract_array()?;
    Ok((
        lines.into_dimensionality::<ndarray::Ix4>()?.to_owned(),
        mask.into_dimensionality::<ndarray::Ix4>()?.to_owned(),
    ))
}

fn preprocess_img(
    img: RawImageView,
    img_processor: &Arc<dyn ImageOp + Send + Sync>,
    input_size: (u32, u32),
    bgr2rgb: bool,
    half: bool,
) -> anyhow::Result<(Array4<f32>, (f64, f64), u32, u32)> {
    let mut img = RawImageCow::Borrowed(img);
    if bgr2rgb {
        img = RawImageCow::Owned(img_processor.bgr_to_rgb(img.to_owned()));
    }
    let (img_in, ratio, (dw, dh)) =
        letterbox(img, img_processor, input_size, false, false, true, 64)?;
    let img_in = img_in.view();
    let nd = img_in.as_ndarray()?;

    let nd = nd.permuted_axes([2, 0, 1]);
    let nd = nd.slice(s![..;-1, .., ..]);
    let nd = nd.mapv(|v| v as f32 / 255.0);
    let nd = stack(Axis(0), &[nd.view()])?;
    if half {
        todo!("convert to f16")
    }
    Ok((nd, ratio, dw, dh))
}

fn letterbox<'a>(
    mut im: RawImageCow<'a>,
    img_processor: &Arc<dyn ImageOp + Send + Sync>,
    new_shape: (u32, u32),
    auto: bool,
    scale_fill: bool,
    scaleup: bool,
    stride: u32,
) -> anyhow::Result<(RawImageCow<'a>, (f64, f64), (u32, u32))> {
    let im_ = im.view();
    let (w, h) = (im_.width, im_.height);
    let mut r = f64::min(new_shape.0 as f64 / h as f64, new_shape.1 as f64 / w as f64);
    if !scaleup {
        r = 1.0_f64.min(r);
    }
    let mut ratio = (r, r);
    let mut new_unpad = ((w as f64 * r).round() as u32, (h as f64 * r).round() as u32);
    let (mut dw, mut dh) = ((new_shape.1 - new_unpad.0), (new_shape.0 - new_unpad.1));
    if auto {
        dw = dw % stride;
        dh = dh % stride;
    } else if scale_fill {
        dw = 0;
        dh = 0;
        new_unpad = (new_shape.1, new_shape.0);
        ratio = (new_shape.1 as f64 / w as f64, new_shape.0 as f64 / h as f64);
    }

    if new_unpad.0 != w as u32 || new_unpad.1 != h as u32 {
        im = RawImageCow::Owned(img_processor.resize(
            im.view(),
            new_unpad.0 as u16,
            new_unpad.1 as u16,
            Interpolation::Bilinear,
        )?);
    }
    let im_ = im.view();

    let im_height = im_.height;
    let im_width = im_.width;
    if let RawImageCow::Owned(vv) =
        img_processor.add_border_wh(im.view(), im_width + dw as u16, im_height + dh as u16)
    {
        im = RawImageCow::Owned(vv);
    }
    Ok((im, ratio, (dw, dh)))
}

#[cfg(test)]
mod tests {

    use std::sync::Arc;

    use base_util::onnx::all_providers;
    use interface_detector::{Detector as _, PreprocessorOptions};
    use interface_image::{CpuImageProcessor, ImageOp, RawImage};
    use interface_model::{Model as _, ModelLoad as _};

    use crate::CtdDetector;

    #[tokio::test]
    async fn load_unload() {
        let data = CtdDetector::new(Arc::new(all_providers()));
        data.load().await.expect("failed to load model");
        data.unload().await;
    }

    #[tokio::test]
    async fn run() {
        let data = CtdDetector::new(Arc::new(all_providers()));
        let cpu_image_processor =
            Arc::new(CpuImageProcessor::default()) as Arc<dyn ImageOp + Send + Sync>;
        data.load().await.expect("Failed to load data");
        let img = RawImage::new("./imgs/232265329-6a560438-e887-4f7f-b6a1-a61b8648f781.png")
            .expect("Failed to load image");
        let (v, _) = data
            .detect(
                &img,
                PreprocessorOptions::default(),
                Default::default(),
                &cpu_image_processor,
            )
            .await
            .expect("failed to detect");
        println!("{:?}", v);
    }
}
