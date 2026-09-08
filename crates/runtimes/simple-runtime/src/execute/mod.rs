mod colorizer;
mod detector;
mod dict;
mod inpainter;
mod mask_refinement;
mod ocr;
mod tagger;
mod textline_merge;
mod translator;
mod upscaler;

use std::{path::PathBuf, ptr, sync::Arc, time::Instant};

macro_rules! timed {
    ($name:literal, $expr:expr) => {{
        let __t = Instant::now();
        let __v = $expr;
        eprintln!("PERF {} {}", $name, __t.elapsed().as_millis());
        __v
    }};
}

use export::Export;
use image::DynamicImage;
use interface_image::{CpuImageProcessor, ImageOp, RawImage};

use crate::{
    debug::{bbox::render_bboxes, save_img, save_json, save_mask, textblocks::render_textblocks},
    settings::Settings,
    setup::Models,
};

pub type ImageProcessor = Arc<dyn ImageOp + Sync + Send>;

impl Models {
    pub async fn execute(
        &mut self,
        img: DynamicImage,
        config: &Settings,
        debug_path: Option<PathBuf>,
        mask_out: Option<PathBuf>,
    ) -> anyhow::Result<Option<Export>> {
        let ip = Arc::new(CpuImageProcessor::default()) as ImageProcessor;
        let (img, alpha) = RawImage::rgba(img);
        let img = timed!(
            "colorizer",
            self.run_colorizer(img, &config.colorizer, &ip).await?
        );
        let (img, alpha) = timed!(
            "upscaler",
            self.run_upscaler(img, alpha, config.upscaler, &ip).await?
        );

        if let Some(debug_path) = &debug_path {
            save_json(config, &debug_path.join("0_config.json"))?;
            save_img(&img, &debug_path.join("0_input.png"))?;
        }

        let (areas, mask) = timed!(
            "detector",
            self.run_detector(&img, &config.detector, &ip).await?
        );
        if let Some(debug_path) = &debug_path {
            save_mask(&mask, &debug_path.join("1_mask_raw.png"))?;
            save_json(&areas, &debug_path.join("1_quadrilateral.json"))?;
            render_bboxes(&img, &areas, debug_path)?;
        }
        if areas.is_empty() {
            write_mask(&mask, &mask_out)?;
            return passthrough_export(img, alpha);
        }

        let areas = areas.into_iter().map(to_mutex).collect::<Vec<_>>();
        let upscaled_img = img;

        let textlines = timed!(
            "ocr",
            self.run_ocr(&upscaled_img, &areas, &config.ocr, &debug_path, &ip)
                .await?
        );

        if textlines.is_empty() {
            write_mask(&mask, &mask_out)?;
            return passthrough_export(upscaled_img, alpha);
        }

        if let Some(debug_path) = &debug_path {
            save_json(&textlines, &debug_path.join("2_quadrilateral.json"))?;
        }

        let textblocks = timed!(
            "merge",
            self.run_textline_merge(
                &textlines,
                &upscaled_img,
                &config.ocr,
                &config.translator,
                &config.render,
            )?
        );
        if textblocks.is_empty() {
            write_mask(&mask, &mask_out)?;
            return passthrough_export(upscaled_img, alpha);
        }

        if let Some(debug_path) = &debug_path {
            save_json(&textblocks, &debug_path.join("3_textblock.json"))?;
            render_textblocks(&upscaled_img, &textblocks, debug_path)?;
        }

        let textblocks = timed!(
            "pre_dict",
            self.run_pre_dict(textblocks, &config.translator)?
        );
        if let Some(debug_path) = &debug_path {
            if config.translator.pre_dict.is_some() {
                save_json(
                    &textlines,
                    &debug_path.join("3_textblock_predict_applied.json"),
                )?;
            }
        }

        let tags = timed!(
            "tagger",
            self.run_tagger(&upscaled_img, &config.tagger, config.translator.mode, &ip)
                .await
        );
        if let (Some(debug_path), Some(tags)) = (&debug_path, &tags) {
            std::fs::write(debug_path.join("3_tags.txt"), tags)?;
        }

        let textblocks = timed!(
            "translate",
            self.run_translators(textblocks, &config.translator, tags.as_deref())
                .await?
        );

        if let Some(debug_path) = &debug_path {
            save_json(
                &textblocks,
                &debug_path.join("4_textblocks_translated.json"),
            )?;
        }

        let textblocks = timed!(
            "post_dict",
            self.run_post_dict(textblocks, &config.translator)?
        );

        let mask_refined = timed!(
            "mask",
            Models::run_mask_refinement(
                &upscaled_img,
                &mask,
                &textblocks,
                &config.mask_refinement,
                &ip,
            )?
        );

        write_mask(&mask_refined, &mask_out)?;
        if let Some(debug_path) = &debug_path {
            save_mask(&mask_refined, &debug_path.join("4_mask_refined.png"))?;
        }

        let upscaled_img = Arc::new(upscaled_img);

        let (inpainted, mask) = timed!(
            "inpaint",
            self.run_inpainter(&upscaled_img, mask, mask_refined, &config.inpainter, &ip)
                .await?
        );

        let inpainted = inpainted.add_a(mask.data);
        if let Some(debug_path) = &debug_path {
            let mut img = upscaled_img.as_ref().clone();
            img.apply_filter(&inpainted, |a, b| unsafe {
                if *b.get_unchecked(3) > 128 {
                    ptr::copy_nonoverlapping(b.as_ptr(), a.as_mut_ptr(), 3);
                }
            });
            save_img(&img, &debug_path.join("5_inpainted.png"))?;
        }

        Ok(Some(Export::new(
            match alpha {
                Some(a) => upscaled_img.as_ref().clone().add_a(a),
                None => upscaled_img.as_ref().clone(),
            }
            .to_image()?,
            inpainted.to_image()?,
            textblocks,
            None,
        )))
    }
}

fn write_mask(mask: &interface_image::Mask, path: &Option<PathBuf>) -> anyhow::Result<()> {
    match path {
        Some(p) => save_mask(mask, p),
        None => Ok(()),
    }
}

fn passthrough_export(img: RawImage, alpha: Option<Vec<u8>>) -> anyhow::Result<Option<Export>> {
    let img = match alpha {
        Some(a) => img.add_a(a),
        None => img,
    };
    let dyn_img = img.to_image()?;
    let overlay =
        DynamicImage::ImageRgba8(image::RgbaImage::new(dyn_img.width(), dyn_img.height()));
    Ok(Some(Export::new(dyn_img, overlay, vec![], None)))
}

fn to_mutex<T>(areas: T) -> Arc<parking_lot::Mutex<T>> {
    Arc::new(parking_lot::Mutex::new(areas))
}
