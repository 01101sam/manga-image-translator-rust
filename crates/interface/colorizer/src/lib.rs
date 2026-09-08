use std::sync::Arc;

use interface_image::{ImageOp, RawImage};

#[derive(Clone, Copy, Debug)]
pub struct ColorizerOptions {
    pub colorization_size: i32,
    pub denoise_sigma: i32,
}

impl Default for ColorizerOptions {
    fn default() -> Self {
        Self {
            colorization_size: 576,
            denoise_sigma: 30,
        }
    }
}

#[async_trait::async_trait]
pub trait Colorizer {
    async fn colorize(
        &self,
        image: &RawImage,
        options: ColorizerOptions,
        img_processor: &Arc<dyn ImageOp + Send + Sync>,
    ) -> anyhow::Result<RawImage>;
}

pub struct NoneColorizer;

#[async_trait::async_trait]
impl Colorizer for NoneColorizer {
    async fn colorize(
        &self,
        image: &RawImage,
        _: ColorizerOptions,
        _: &Arc<dyn ImageOp + Send + Sync>,
    ) -> anyhow::Result<RawImage> {
        Ok(image.clone())
    }
}

#[cfg(test)]
mod tests {
    use interface_image::{CpuImageProcessor, ImageOp};

    use super::*;

    #[tokio::test]
    async fn none_is_identity() {
        let img = RawImage {
            data: vec![10, 20, 30, 40, 50, 60],
            width: 2,
            height: 1,
            channels: 3,
        };
        let ip = Arc::new(CpuImageProcessor::default()) as Arc<dyn ImageOp + Send + Sync>;
        let out = NoneColorizer
            .colorize(&img, ColorizerOptions::default(), &ip)
            .await
            .unwrap();
        assert_eq!(out.data, img.data);
        assert_eq!(out.width, img.width);
        assert_eq!(out.height, img.height);
    }
}
