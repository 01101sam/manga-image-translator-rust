use interface_image::RawImage;
use interface_ocr::QuadrilateralInfo;
use log::info;
use textline_merge::TextBlock;

use crate::{
    settings::{OCRSettings, RenderSettings, TranslatorSettings},
    setup::Models,
};

impl Models {
    pub fn run_textline_merge(
        &self,
        textlines: &[QuadrilateralInfo],
        img: &RawImage,
        config: &OCRSettings,
        config2: &TranslatorSettings,
        render: &RenderSettings,
    ) -> anyhow::Result<Vec<TextBlock>> {
        assert!(!textlines.is_empty());
        info!("Run Textline Merge");
        let blocks = textline_merge::dispatch_main(
            textlines,
            img.width,
            img.height,
            config.post_processing.min_text_length,
            config.post_processing.prob,
            config2.filter_lang.iter().map(|v| v.0).collect(),
            &config.post_processing.filter_text,
            &self.lang_detector,
        )?;
        Ok(textline_merge::sort_regions(
            blocks,
            render.rtl,
            Some(img),
            render.force_simple_sort,
        ))
    }
}
