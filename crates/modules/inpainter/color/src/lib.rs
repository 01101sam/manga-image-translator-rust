use std::{collections::HashMap, sync::Arc};

use interface_inpainter::{colorize_mask_area, Inpainter, InpainterOptions};
use interface_model::Model;
use tokio::sync::Mutex;

pub struct ColorInpainter {
    loaded: Arc<Mutex<bool>>,
}

impl Default for ColorInpainter {
    fn default() -> Self {
        Self::new()
    }
}

impl ColorInpainter {
    pub fn new() -> Self {
        Self {
            loaded: Arc::new(Mutex::new(true)),
        }
    }
}

#[async_trait::async_trait]
impl Model for ColorInpainter {
    fn name(&self) -> &'static str {
        "color"
    }

    fn kind(&self) -> &'static str {
        "inpainter"
    }

    fn models(&self) -> std::collections::HashMap<&'static str, interface_model::ModelSource> {
        HashMap::new()
    }

    async fn unload(&self) {
        *self.loaded.lock().await = false;
    }

    async fn loaded_(&self) -> bool {
        *self.loaded.lock().await
    }

    async fn reload_(&self) -> anyhow::Result<()> {
        *self.loaded.lock().await = true;
        Ok(())
    }
}

#[async_trait::async_trait]
impl Inpainter for ColorInpainter {
    async fn inpaint(
        &self,
        image: &interface_image::RawImage,
        mask: interface_image::Mask,
        options: InpainterOptions,
        _: &Arc<dyn interface_image::ImageOp + Send + Sync>,
    ) -> anyhow::Result<interface_image::RawImage> {
        Ok(colorize_mask_area(image.clone(), &mask, options.color))
    }
}

pub struct OriginalInpainter {
    loaded: Arc<Mutex<bool>>,
}

impl Default for OriginalInpainter {
    fn default() -> Self {
        Self::new()
    }
}

impl OriginalInpainter {
    pub fn new() -> Self {
        Self {
            loaded: Arc::new(Mutex::new(true)),
        }
    }
}

#[async_trait::async_trait]
impl Model for OriginalInpainter {
    fn name(&self) -> &'static str {
        "original"
    }

    fn kind(&self) -> &'static str {
        "inpainter"
    }

    fn models(&self) -> HashMap<&'static str, interface_model::ModelSource> {
        HashMap::new()
    }

    async fn unload(&self) {
        *self.loaded.lock().await = false;
    }

    async fn loaded_(&self) -> bool {
        *self.loaded.lock().await
    }

    async fn reload_(&self) -> anyhow::Result<()> {
        *self.loaded.lock().await = true;
        Ok(())
    }
}

#[async_trait::async_trait]
impl Inpainter for OriginalInpainter {
    async fn inpaint(
        &self,
        image: &interface_image::RawImage,
        _: interface_image::Mask,
        _: InpainterOptions,
        _: &Arc<dyn interface_image::ImageOp + Send + Sync>,
    ) -> anyhow::Result<interface_image::RawImage> {
        Ok(image.clone())
    }
}

#[cfg(test)]
mod tests {
    use interface_image::{CpuImageProcessor, ImageOp, Mask, RawImage};

    use super::*;

    fn sample() -> (RawImage, Mask, Arc<dyn ImageOp + Send + Sync>) {
        let img = RawImage {
            data: vec![1, 2, 3, 4, 5, 6],
            width: 2,
            height: 1,
            channels: 3,
        };
        let mask = Mask {
            data: vec![255, 0],
            width: 2,
            height: 1,
        };
        let ip = Arc::new(CpuImageProcessor::default()) as Arc<dyn ImageOp + Send + Sync>;
        (img, mask, ip)
    }

    #[tokio::test]
    async fn color_fills_masked_pixels() {
        let (img, mask, ip) = sample();
        let out = ColorInpainter::new()
            .inpaint(
                &img,
                mask,
                InpainterOptions {
                    color: [9, 9, 9],
                    ..Default::default()
                },
                &ip,
            )
            .await
            .unwrap();
        assert_eq!(out.data, vec![9, 9, 9, 4, 5, 6]);
    }

    #[tokio::test]
    async fn original_passthrough() {
        let (img, mask, ip) = sample();
        let out = OriginalInpainter::new()
            .inpaint(&img, mask, Default::default(), &ip)
            .await
            .unwrap();
        assert_eq!(out.data, img.data);
    }
}
