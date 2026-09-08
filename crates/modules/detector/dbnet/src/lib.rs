use std::{
    ops::Deref,
    sync::{Arc, Mutex},
};

use base_util::onnx::{new_session, Providers};

use interface_detector::{textlines::Quadrilateral, DefaultOptions, Detector};
use interface_image::{DimType, ImageOp, Interpolation, Mask, RawImageCow};
use interface_model::{
    impl_model_helpers, impl_model_load_helpers, Model, ModelLoad, ModelRead, ModelSource,
    ModelWrap,
};
use log::debug;

use ndarray::{array, Array2, Array3, Array4, ArrayView4, ArrayViewD, Axis};
use opencv::core::BORDER_DEFAULT;
use ort::{session::Session, value::Tensor};
use util::{
    det_arrange::{det_rearrange_forward, det_unarrange, shoud_rearrange},
    opencv::bilateral_filter,
};

use maplit::hashmap;

pub struct DbNetDetector {
    providers: Arc<Vec<Providers>>,
    model: ModelWrap<Mutex<Session>>,
    /// Different model architecture, but based on dbnet
    convnext: bool,
}

impl DbNetDetector {
    ///convnext: Different model architecture, but based on dbnet
    pub fn new(providers: Arc<Vec<Providers>>, convnext: bool) -> Self {
        DbNetDetector {
            providers,
            model: Default::default(),
            convnext,
        }
    }
}

#[async_trait::async_trait]
impl ModelLoad for DbNetDetector {
    impl_model_load_helpers!(model, Mutex<Session>);
    async fn reload(&self) -> anyhow::Result<ModelRead<Self::T>> {
        let p = self.download_model("model", "model.onnx").await?;
        let session = new_session(&self.providers)?.commit_from_file(&p)?;
        *self.model.write().await = Some(Mutex::new(session));
        Ok(self.get_model().await.expect("set before"))
    }
}

impl Model for DbNetDetector {
    impl_model_helpers!("detector", "dbnet", model);

    fn models(&self) -> std::collections::HashMap<&'static str, ModelSource> {
        hashmap! {
            "model"=> ModelSource {
                url: "https://github.com/frederik-uni/manga-image-translator-rust/releases/download/dbnet-v1.0.0/model.onnx",
                hash: "7b348114b09015ce18373049c0ff90ce9a55fd3378cd33fd6209c80d1d04660e",
            }
        }
    }
}

/// Runs the session synchronously on the calling thread. ORT's `RunAsync` path (what
/// `ort-parallel` wraps) executed the first two runs of every session near single-threaded
/// (~10s instead of ~1.6s each) and stayed ~17% slower afterwards.
fn det_batch_forward_default(
    session: &Mutex<Session>,
    batch: ArrayView4<u8>,
) -> anyhow::Result<(Array4<f32>, Array4<f32>)> {
    let batch = batch
        .mapv(|x| x as f32 / 127.5 - 1.0)
        .permuted_axes([0, 3, 1, 2]);
    let tensor = Tensor::from_array(batch)?;
    let mut session = session.lock().expect("session mutex poisoned");
    let outputs = session.run(ort::inputs!["input" => tensor])?;
    let db: ArrayViewD<f32> = outputs["db"].try_extract_array()?;
    let mask: ArrayViewD<f32> = outputs["mask"].try_extract_array()?;
    let db = db.mapv(|x| 1.0 / (1.0 + (-x).exp()));
    Ok((
        db.into_dimensionality::<ndarray::Ix4>()?,
        mask.into_dimensionality::<ndarray::Ix4>()?.to_owned(),
    ))
}

#[async_trait::async_trait]
impl Detector for DbNetDetector {
    async fn infer(
        &self,
        img: RawImageCow<'_>,
        options: DefaultOptions,
        img_processor: &Arc<dyn ImageOp + Send + Sync>,
    ) -> anyhow::Result<(Vec<Quadrilateral>, Mask)> {
        let session = self.load().await?;
        let session = session.deref();

        let (db, mask, shape, ratio_w, ratio_h, pad_w, pad_h) =
            match shoud_rearrange(img.view(), options.detect_size as u32) {
                true => {
                    let img_ = img.view();
                    let shape = (img_.height, img_.width);
                    let (batch_results, rv) = det_rearrange_forward(
                        img.view(),
                        options.detect_size as u32,
                        4,
                        img_processor,
                    )?;
                    let out = batch_results
                        .iter()
                        .map(|batch| det_batch_forward_default(session, batch.view()))
                        .collect::<anyhow::Result<Vec<_>>>()?;
                    let (db, mask) = det_unarrange(out, rv)?;
                    (db, mask, shape, 1.0, 1.0, 0, 0)
                }
                false => {
                    let resized = util::imageproc::resize_aspect_ratio(
                        bilateral_filter(
                            &img.view().as_opencv_mat()?,
                            17,
                            80.0,
                            80.0,
                            BORDER_DEFAULT,
                        )?,
                        options.detect_size as i64,
                        Interpolation::Bilinear,
                        1.0,
                        img_processor,
                    )?;
                    let ratio_h = 1.0 / resized.ratio;
                    let ratio_w = ratio_h;
                    let shape = (resized.img.height, resized.img.width);
                    let img = resized.img.as_ndarray()?.insert_axis(ndarray::Axis(0));
                    let (db, mask) = det_batch_forward_default(session, img)?;
                    (
                        db,
                        mask,
                        shape,
                        ratio_w,
                        ratio_h,
                        resized.pad_w,
                        resized.pad_h,
                    )
                }
            };

        let mask: Array2<f32> = mask
            .index_axis(ndarray::Axis(0), 0)
            .index_axis(ndarray::Axis(0), 0)
            .to_owned();

        debug!("Detection resolution: {}x{}", shape.1, shape.0);

        let det = util::dbnet::SegDetectorRepresenter {
            min_size: 3.0,
            thresh: options.text_threshold as f32,
            box_thresh: options.box_threshold,
            max_candidates: 1000,
            unclip_ratio: options.unclip_ratio,
        };

        let (mut boxes, mut scores) = det.call(
            db.mapv(|v| v).into_dimensionality()?,
            false,
            shape.1,
            shape.0,
        )?;

        let boxes = boxes.remove(0);
        let scores = scores.remove(0);
        let (boxes, scores) = match (boxes, scores) {
            (Some(b), Some(s)) => (b, s),
            _ => {
                return Ok((
                    vec![],
                    Mask {
                        width: 0,
                        height: 0,
                        data: Vec::new(),
                    },
                ))
            }
        };
        let polys = filter_boxes_and_adjust(&boxes, ratio_w, ratio_h);
        let quadrilateral = polys
            .outer_iter()
            .zip(scores)
            .map(|(pts, score)| {
                Quadrilateral::new(
                    pts.outer_iter()
                        .map(|v| (v[0], v[1]))
                        .collect::<Vec<(i64, i64)>>(),
                    score,
                )
            })
            .filter(|v| v.area() >= 16.0)
            .collect::<Vec<_>>();

        let mask = Mask::from(mask.mapv(|v| f32::clamp(v * 255.0, 0.0, 255.0) as u8));
        let t_w = mask.width as usize * 2;
        let t_h = mask.height as usize * 2;
        let mut mask_resized =
            img_processor.resize_mask(mask.view(), t_w, t_h, Interpolation::Bilinear)?;
        let new_mask_width = mask_resized.width - pad_w as DimType;
        let new_mask_height = mask_resized.height - pad_h as DimType;
        if pad_h > 0 || pad_w > 0 {
            mask_resized =
                img_processor.remove_border_mask(mask_resized, new_mask_width, new_mask_height);
        }

        Ok((quadrilateral, mask_resized))
    }
}

fn filter_boxes_and_adjust(boxes: &Array3<i64>, ratio_w: f64, ratio_h: f64) -> Array3<i64> {
    if boxes.is_empty() {
        return Array3::<i64>::zeros((0, 0, 0));
    }
    let boxes = boxes.to_shared();
    let idx = boxes
        .reshape((boxes.shape()[0], boxes.len() / boxes.shape()[0]))
        .sum_axis(Axis(1))
        .mapv(|v| v > 0);
    let indicies = idx
        .iter()
        .enumerate()
        .filter(|(_, b)| **b)
        .map(|(i, _)| i)
        .collect::<Vec<usize>>();
    let polys = boxes.select(Axis(0), &indicies);
    let polys = polys.mapv(|v| v as f64);
    let polys = adjust_result_coordinates(polys, ratio_w, ratio_h, 1.0);
    polys.mapv(|v| v as i64)
}

fn adjust_result_coordinates(
    polys: Array3<f64>,
    ratio_w: f64,
    ratio_h: f64,
    ratio_net: f64,
) -> Array3<f64> {
    let scale = array![ratio_w * ratio_net, ratio_h * ratio_net];
    polys * &scale
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::{DbNetDetector, DefaultOptions};
    use base_util::onnx::all_providers;
    use interface_detector::{Detector, PreprocessorOptions};
    use interface_image::{CpuImageProcessor, ImageOp, RawImage};
    use interface_model::{Model as _, ModelLoad as _};

    #[tokio::test]
    async fn load_unload() {
        let data = DbNetDetector::new(Arc::new(all_providers()), false);
        data.load().await.expect("failed to load model");
        data.unload().await;
    }

    #[tokio::test]
    async fn run() {
        let data = DbNetDetector::new(Arc::new(all_providers()), false);
        let cpu_image_processor =
            Arc::new(CpuImageProcessor::default()) as Arc<dyn ImageOp + Send + Sync>;
        data.detect(
            &RawImage::new("./imgs/232265329-6a560438-e887-4f7f-b6a1-a61b8648f781.png")
                .expect("Failed to load image"),
            PreprocessorOptions::default(),
            DefaultOptions::default(),
            &cpu_image_processor,
        )
        .await
        .expect("failed to detect");
    }
}
