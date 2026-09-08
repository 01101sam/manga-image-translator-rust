use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Default, Hash, PartialEq, Eq, Copy, Clone, JsonSchema, Debug)]
pub enum Tagger {
    #[default]
    Pixai,
}

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(default)]
/// Image tagging that feeds the `<tags>` block of the LLM translator
pub struct TaggerSettings {
    /// `None` translates without tags
    pub tagger: Option<Tagger>,
    /// Minimum probability for general tags
    pub general_threshold: f32,
    /// Minimum probability for character tags (and the franchises derived from them)
    pub character_threshold: f32,
}

impl Default for TaggerSettings {
    fn default() -> Self {
        Self {
            tagger: Some(Tagger::Pixai),
            general_threshold: 0.3,
            character_threshold: 0.85,
        }
    }
}
