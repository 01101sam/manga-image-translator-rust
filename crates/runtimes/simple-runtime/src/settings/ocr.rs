use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum_macros::EnumIter;

#[derive(
    Serialize, Deserialize, Default, EnumIter, Hash, PartialEq, Eq, Copy, Clone, JsonSchema, Debug,
)]
pub enum OCR {
    MangaOcr,
    Native,
    Ctc48px,
    #[default]
    Ocr48px,
}

#[derive(Serialize, Deserialize, Default, JsonSchema)]
#[serde(default)]
pub struct OCRSettings {
    /// Optical character recognition (OCR) model to use
    pub ocr: OCR,
    /// Use bbox merge when Manga OCR inference.
    /// todo: not used
    use_mocr_merge: bool,

    #[serde(flatten)]
    pub post_processing: PostProcessingSettings,
}

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct PostProcessingSettings {
    /// Minimum text length of a text region
    pub min_text_length: usize,
    /// Filter regions by their text with a regex. Example usage: '.*badtext.*'
    pub filter_text: Vec<String>,
    /// Minimum probability of a text region to be considered valid. If None, uses the model default
    pub prob: f64,
    /// Skip non-bubble text. 1 to 50, 0 disables
    pub ignore_bubble: u8,
}

impl Default for PostProcessingSettings {
    fn default() -> Self {
        Self {
            min_text_length: 1,
            filter_text: Vec::new(),
            prob: 0.2,
            ignore_bubble: 0,
        }
    }
}
