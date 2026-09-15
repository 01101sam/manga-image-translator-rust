use interface_image::{Mask, RawImage};
use interface_inpainter::InpainterOptions;
use log::info;
use util::spawn_blocking;

use crate::{
    execute::ImageProcessor,
    settings::{self, InpainterSettings},
    setup::Models,
};

impl Models {
    pub async fn run_inpainter(
        &self,
        img: &RawImage,
        original_mask: Mask,
        mask: Mask,
        config: &InpainterSettings,
        ip: &ImageProcessor,
    ) -> anyhow::Result<(RawImage, Mask)> {
        info!("Run Inpainter: {:?}", config.inpainter);
        let mask_ = spawn_blocking!(|| match config.mask {
            settings::Mask::Mask => original_mask,
            settings::Mask::RefinedMask => mask.clone(),
            settings::Mask::Both => ip.mask_func(original_mask, mask.clone(), |f, s| {
                if f > 128 && s > 128 {
                    255
                } else {
                    0
                }
            }),
        })?;
        let inpainted = self
            .get_inpainter(config.inpainter)
            .inpaint(
                img,
                mask,
                InpainterOptions {
                    inpainting_size: config.inpainting_size,
                    color: config.inpaint_color,
                },
                &ip,
            )
            .await?;

        Ok((inpainted, mask_))
    }
}
