use interface_colorizer::ColorizerOptions;
use interface_image::RawImage;
use log::info;

use crate::{
    execute::ImageProcessor, settings::ColorizerSettings, setup::Models,
};

impl Models {
    pub async fn run_colorizer(
        &self,
        img: RawImage,
        config: &ColorizerSettings,
        ip: &ImageProcessor,
    ) -> anyhow::Result<RawImage> {
        info!("Run Colorizer: {:?}", config.colorizer);
        self.get_colorizer(config.colorizer)
            .colorize(
                &img,
                ColorizerOptions {
                    colorization_size: config.colorization_size,
                    denoise_sigma: config.denoise_sigma,
                },
                ip,
            )
            .await
    }
}
