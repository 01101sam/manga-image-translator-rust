use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum_macros::EnumIter;

#[derive(
    Serialize, Deserialize, Default, EnumIter, Hash, PartialEq, Eq, Copy, Clone, JsonSchema, Debug,
)]
pub enum Colorizer {
    #[default]
    None,
}

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ColorizerSettings {
    pub colorizer: Colorizer,
    pub colorization_size: i32,
    pub denoise_sigma: i32,
}

impl Default for ColorizerSettings {
    fn default() -> Self {
        Self {
            colorizer: Colorizer::None,
            colorization_size: 576,
            denoise_sigma: 30,
        }
    }
}
