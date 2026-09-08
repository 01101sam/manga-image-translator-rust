pub mod textlines;

use std::sync::Arc;

use base_util::RawSerializable;
use interface_image::{ImageOp, Mask, RawImage, RawImageCow};
use interface_model::Model;
use log::{debug, info};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::textlines::Quadrilateral;

#[derive(Default, Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(default)]
pub struct PreprocessorOptions {
    /// Invert the image colors for detection. Might improve detection.
    pub invert: bool,
    /// Applies gamma correction for detection. Might improve detection.
    pub gamma_correct: bool,
    /// Rotate the image for detection. Might improve detection.
    pub rotate: bool,
    /// Rotate the image for detection to prefer vertical textlines. Might improve detection.
    pub auto_rotate: bool,
}

impl PreprocessorOptions {
    pub fn set_auto_rotate(mut self, auto_rotate: bool) -> Self {
        self.auto_rotate = auto_rotate;
        self
    }
}

// pub fn default_detect(
//     detector: &mut dyn Detector,
//     image: &RawImage,
//     pre_options: PreprocessorOptions,
//     options: &dyn Any,
//     img_processor: &Arc<dyn ImageOp + Send + Sync>,
// ) -> anyhow::Result<(Vec<Quadrilateral>, Mask)> {

// }
//
//

#[async_trait::async_trait]
pub trait Detector: Model {
    async fn detect(
        &self,
        image: &RawImage,
        pre_processor_options: PreprocessorOptions,
        options: DefaultOptions,
        img_processor: &Arc<dyn ImageOp + Send + Sync>,
    ) -> anyhow::Result<(Vec<Quadrilateral>, Mask)> {
        let img_h = image.height as i64;
        let pp_opt = pre_processor_options;
        // Automatically add border if image too small (instead of simply resizing due to them more likely containing large fonts)
        let mut add_border = None;
        if image.width.min(image.height) < 400 {
            add_border = Some((image.width, image.height));
            debug!("Adding border")
        }
        let mut img = img_processor.add_border(image.view(), 400);
        if pp_opt.rotate {
            debug!("Rotating image");
            img = RawImageCow::Owned(img_processor.rotate_right(img.view()));
        }

        if pp_opt.invert {
            debug!("Adding inversion");
            img = RawImageCow::Owned(img_processor.invert(img.to_owned()));
        }

        if pp_opt.gamma_correct {
            debug!("Adding gamma correction");
            img = RawImageCow::Owned(img_processor.gamma_correction(img.view()));
        }

        let (mut textlines, mut mask) = self.infer(img, options, img_processor).await?;

        if pp_opt.auto_rotate {
            let rerun = if !textlines.is_empty() {
                textlines.len() * 2 >= textlines.iter().map(|v| v.aspect_ratio() > 1.0).count()
            } else {
                true
            };

            if rerun {
                info!("Rerunning detection with 90° rotation");
                return self
                    .detect(
                        image,
                        pre_processor_options.set_auto_rotate(false),
                        options,
                        img_processor,
                    )
                    .await;
            }
        }

        if let Some((w, h)) = add_border {
            debug!("Removing border from mask");

            mask = img_processor.remove_border_mask(mask, w, h);
        }

        if pp_opt.rotate {
            debug!("Rotating mask and textlines");
            mask = img_processor.rotate_left_mask(mask);
            textlines = textlines
                .into_iter()
                .map(|v| {
                    Quadrilateral::new(
                        v.pts()
                            .iter()
                            .map(|&point| {
                                let new_x = point.y;
                                let new_y = -point.x + img_h;
                                (new_x, new_y)
                            })
                            .collect(),
                        v.score(),
                    )
                })
                .collect::<Vec<_>>();
        }

        let ow = image.width as i64;
        let oh = image.height as i64;
        textlines = textlines
            .into_iter()
            .filter_map(|q| q.clip_to(ow, oh))
            .collect();

        Ok((textlines, mask))
    }
    async fn infer(
        &self,
        img: RawImageCow<'_>,
        options: DefaultOptions,
        img_processor: &Arc<dyn ImageOp + Send + Sync>,
    ) -> anyhow::Result<(Vec<Quadrilateral>, Mask)>;
}

#[derive(Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[repr(C)]
#[serde(default)]
pub struct DefaultOptions {
    /// Text detector used for creating a text mask from an image
    /// TODO: guide
    pub detect_size: u64,
    /// How much to extend text skeleton to form bounding box
    /// smaller values = smaller text skeleton.
    /// to small = more false negatives/partial detections
    /// larger values = bigger text skeleton detections .
    /// to big =  more false positives/Multiple close text lines/words may be merged
    /// Suggested values:
    /// - `1.0 – 1.5`: Use for tight text layouts, well-separated characters or lines, high-resolution images.
    /// - `1.5 – 2.0`: General-purpose setting. Provides a good balance between recall and precision.
    /// - `2.0 – 2.5`: Use when text is thin, faint, or sparse—e.g., scanned documents or light fonts.
    /// - `> 2.5`: Rarely needed. May cause nearby text instances to merge or overlap.
    pub unclip_ratio: f64,
    /// Threshold for text detection
    /// smaller values = more detections + more false positives
    /// larger values = fewer detections + more false negatives
    /// allowed range is from 0.0 to 1.0
    pub text_threshold: f64,
    /// Threshold for bbox generation
    /// to small = more false positives/ noise, background artifacts, or partial text.
    /// to big = false negatives/ actual text that had slightly lower confidence is discarded.
    /// allowed range is from 0.0 to 1.0
    pub box_threshold: f64,
}

impl RawSerializable for DefaultOptions {}
impl Default for DefaultOptions {
    fn default() -> Self {
        Self {
            detect_size: 2048,
            unclip_ratio: 2.3,
            text_threshold: 0.5,
            box_threshold: 0.7,
        }
    }
}
