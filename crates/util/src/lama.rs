use std::sync::Arc;

use interface_image::{DimType, ImageOp, Mask, RawImage, RawImageCow, RawImageView};

pub fn resize_keep_aspect(
    img: RawImageView,
    size: u16,
    img_processor: &Arc<dyn ImageOp + Send + Sync>,
) -> anyhow::Result<RawImage> {
    let ratio = size as f64 / img.width.max(img.height) as f64;
    let new_width = img.width as f64 * ratio;
    let new_height = img.height as f64 * ratio;

    img_processor.resize(
        img,
        new_width as DimType,
        new_height as DimType,
        interface_image::Interpolation::BilinearExact,
    )
}

pub fn resize_keep_aspect_mask(
    img: Mask,
    size: u16,
    img_processor: &Arc<dyn ImageOp + Send + Sync>,
) -> anyhow::Result<Mask> {
    let ratio = size as f64 / img.width.max(img.height) as f64;
    let new_width = img.width as f64 * ratio;
    let new_height = img.height as f64 * ratio;

    img_processor.resize_mask(
        img.view(),
        new_width as usize,
        new_height as usize,
        interface_image::Interpolation::BilinearExact,
    )
}

pub fn lama_resize_image<'a>(
    image: RawImageView<'a>,
    mut mask: Mask,
    inpainting_size: u16,
    img_processor: &Arc<dyn ImageOp + Send + Sync>,
) -> anyhow::Result<(RawImageCow<'a>, Mask)> {
    let w = image.width;
    let h = image.height;
    let mut image = RawImageCow::Borrowed(image);
    if w.max(h) > inpainting_size {
        image = RawImageCow::Owned(resize_keep_aspect(
            image.view(),
            inpainting_size,
            img_processor,
        )?);
        mask = resize_keep_aspect_mask(mask, inpainting_size, img_processor)?;
    }
    Ok((image, mask))
}

pub fn lama_pad_dim(n: u16) -> u16 {
    // FFC 在 /8 特征图上做 FFT；只 pad 到 8 时该层可以是奇数（如 1464/8=183），
    // ONNX Add 会变成 182 vs 183。pad 到 16 保证 /8 为偶数。
    const PAD: u16 = 16;
    let r = n % PAD;
    if r == 0 {
        n
    } else {
        n + PAD - r
    }
}

pub fn lama_add_border(
    image: RawImage,
    mask: Mask,
    img_processor: &Arc<dyn ImageOp + Send + Sync>,
) -> (RawImage, Mask, u16, u16) {
    let new_w = lama_pad_dim(image.width);
    let new_h = lama_pad_dim(image.height);
    let (image, mask) = lama_pad_canvas(image, mask, new_w, new_h, img_processor);
    (image, mask, new_w, new_h)
}

/// Zero-pads image and mask at the bottom/right to `new_w` x `new_h` (no-op when already that size).
pub fn lama_pad_canvas(
    mut image: RawImage,
    mut mask: Mask,
    new_w: u16,
    new_h: u16,
    img_processor: &Arc<dyn ImageOp + Send + Sync>,
) -> (RawImage, Mask) {
    if new_h != image.height || new_w != image.width {
        if let RawImageCow::Owned(o) = img_processor.add_border_wh(image.view(), new_w, new_h) {
            image = o;
        }

        let mut m = RawImage {
            data: mask.data,
            width: mask.width,
            height: mask.height,
            channels: 1,
        };
        if let RawImageCow::Owned(o) = img_processor.add_border_wh(m.view(), new_w, new_h) {
            m = o;
        }
        mask = Mask {
            data: m.data,
            width: m.width,
            height: m.height,
        };
    }
    (image, mask)
}

#[cfg(test)]
mod tests {
    use super::lama_pad_dim;

    #[test]
    fn pad_keeps_ffc_spatial_even() {
        for n in [1u16, 7, 8, 9, 15, 16, 1463, 1464, 1472] {
            let p = lama_pad_dim(n);
            assert_eq!(p % 16, 0, "n={n}");
            assert_eq!((p / 8) % 2, 0, "n={n} p={p}");
        }
        assert_eq!(lama_pad_dim(1464), 1472);
        assert_eq!(lama_pad_dim(8), 16);
    }
}
