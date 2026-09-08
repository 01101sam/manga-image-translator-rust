use interface_detector::{DefaultOptions, PreprocessorOptions};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum_macros::EnumIter;

#[derive(
    Serialize, Deserialize, Default, EnumIter, Hash, PartialEq, Eq, Copy, Clone, JsonSchema, Debug,
)]
pub enum Detector {
    #[default]
    DBNet,
    // DBNetConvNext,
    Ctd,
}

#[derive(Serialize, Deserialize, Default, JsonSchema)]
#[serde(default)]
pub struct DetectorSettings {
    /// Text detector used for creating a text mask from an image
    pub detector: Detector,
    /// General Options to apply before detection
    pub preprocessor: PreprocessorOptions,
    /// Detector specific options
    pub options: DefaultOptions,
    // todo: skip mask generation
}
