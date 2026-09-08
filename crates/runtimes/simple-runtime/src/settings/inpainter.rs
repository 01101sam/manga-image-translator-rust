use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum_macros::EnumIter;

#[derive(
    Serialize, Deserialize, Default, EnumIter, Hash, PartialEq, Eq, Copy, Clone, JsonSchema, Debug,
)]
pub enum Inpainter {
    LamaAot,
    #[default]
    LamaLarge,
    LamaMpe,
    None,
    Original,
}

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct InpainterSettings {
    /// Inpainting model to use
    pub inpainter: Inpainter,
    /// Size of image used for inpainting (too large will result in OOM)
    pub inpainting_size: u16,
    /// If no ai is used for inpainting than use this color
    pub inpaint_color: [u8; 3],
    /// this is not the mask that is given to the inpainter, but what part of the inpainted image should be extracted(overlayed on the orignal image)
    pub mask: Mask,
}
#[derive(Serialize, Deserialize, JsonSchema, Default)]
pub enum Mask {
    /// The detected mask needs to be refined to remove artifacts when inpainting. Its alsmost a perfect match to the actual letters, but does contain some noise
    Mask,
    /// The refined mask is used for inpainting, but a bit more dilated & without the noise
    #[default]
    RefinedMask,
    /// this is cuts out the parts which are in both mask and refined mask(filters noise)
    Both,
}

impl Default for InpainterSettings {
    fn default() -> Self {
        Self {
            inpainter: Default::default(),
            inpainting_size: 2048,
            inpaint_color: [255; 3],
            mask: Default::default(),
        }
    }
}
