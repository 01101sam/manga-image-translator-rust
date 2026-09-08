use interface_image::RawImage;
use log::{info, warn};
use pixai_tagger::TaggerOptions;

use crate::{
    execute::ImageProcessor,
    settings::{Tagger, TaggerSettings, TranslatorMode},
    setup::Models,
};

impl Models {
    /// The body of the LLM translator's `<tags>` block. `None` when tagging is off, when no LLM
    /// call will happen, or when the tagger is unavailable.
    pub async fn run_tagger(
        &mut self,
        img: &RawImage,
        config: &TaggerSettings,
        mode: TranslatorMode,
        ip: &ImageProcessor,
    ) -> Option<String> {
        let Some(Tagger::Pixai) = config.tagger else {
            return None;
        };
        if mode != TranslatorMode::Llm {
            return None;
        }
        let tagger = self.tagger.as_ref()?;
        info!("Run Tagger: {:?}", Tagger::Pixai);
        let options = TaggerOptions {
            general_threshold: config.general_threshold,
            character_threshold: config.character_threshold,
        };
        match tagger.tag(img, options, ip).await {
            Ok(tags) => Some(tags.to_string()),
            Err(e) => {
                // Most likely the model is not downloadable yet; retrying would cost a failed
                // download on every page, so the rest of the run translates without tags.
                warn!("tagger unavailable, translating without tags: {e:#}");
                self.tagger = None;
                None
            }
        }
    }
}
