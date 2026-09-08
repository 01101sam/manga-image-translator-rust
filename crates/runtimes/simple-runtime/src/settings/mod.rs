mod colorizer;
mod detector;
mod inpainter;
mod mask_refinement;
mod ocr;
mod render;
mod translator;
mod upscaler;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use crate::settings::{
    colorizer::Colorizer, colorizer::ColorizerSettings, detector::Detector,
    detector::DetectorSettings, inpainter::Inpainter, inpainter::InpainterSettings,
    inpainter::Mask, mask_refinement::MaskRefinementSettings, ocr::OCRSettings, ocr::OCR,
    render::Alignment, render::Direction, render::RenderSettings, render::Renderer,
    translator::Target, translator::Translation, translator::Translator,
    translator::TranslatorSettings, upscaler::Upscaler, upscaler::UpscalerSettings,
};

#[derive(Serialize, Deserialize, Default, JsonSchema)]
#[serde(default)]
/// Settings for the simple runtime
pub struct Settings {
    /// Settings for the upscaler module
    pub upscaler: UpscalerSettings,

    /// Settings for the colorizer module
    pub colorizer: ColorizerSettings,

    /// Settings for the detector module
    pub detector: DetectorSettings,

    /// Settings for the OCR module
    pub ocr: OCRSettings,

    /// Settings for the mask refinement
    pub mask_refinement: MaskRefinementSettings,

    /// Settings for the inpainter module
    pub inpainter: InpainterSettings,

    /// Settings for the translator module
    pub translator: TranslatorSettings,

    /// Settings for the render module
    pub render: RenderSettings,
}

#[cfg(test)]
mod tests {
    use std::{fs::File, io::Write};

    use crate::settings::Settings;

    #[test]
    fn generate_schema() {
        let schema = schemars::schema_for!(Settings);
        let json = serde_json::to_string_pretty(&schema).unwrap();
        File::create("../../../docs/schema.json")
            .unwrap()
            .write_all(json.as_bytes())
            .unwrap();
        File::create("../../../docs/example.json")
            .unwrap()
            .write_all(
                serde_json::to_string(&Settings::default())
                    .unwrap()
                    .as_bytes(),
            )
            .unwrap();
    }
}
