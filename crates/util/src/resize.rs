use anyhow::anyhow;
use image::{DynamicImage, RgbImage};
use interface_detector::textlines::{MyPoint, Quadrilateral};
use interface_image::RawImage;
use opencv::{
    calib3d::{find_homography, RANSAC},
    core::{
        no_array, rotate, Mat, Point2f, Scalar, Size, Vector, BORDER_CONSTANT,
        ROTATE_90_COUNTERCLOCKWISE,
    },
    imgproc::INTER_LINEAR,
};

pub fn clamp_crop(
    x1: i64,
    y1: i64,
    x2: i64,
    y2: i64,
    width: i64,
    height: i64,
) -> Option<(u32, u32, u32, u32)> {
    let x1 = x1.clamp(0, width);
    let y1 = y1.clamp(0, height);
    let x2 = x2.clamp(x1, width);
    let y2 = y2.clamp(y1, height);
    (x2 > x1 && y2 > y1).then_some((x1 as u32, y1 as u32, (x2 - x1) as u32, (y2 - y1) as u32))
}

pub fn get_transformed_region(
    q: &Quadrilateral,
    img: &RgbImage,
    text_height: u32,
) -> anyhow::Result<Option<Mat>> {
    let [l1a, l1b, l2a, l2b] = <[MyPoint<f64>; 4]>::try_from(
        q.structure()
            .into_iter()
            .map(|v| v.to_f64())
            .collect::<Vec<_>>(),
    )
    .map_err(|v| anyhow!("invalid array len: {}", v.len()))?;
    let im_w = img.width() as i64;
    let im_h = img.height() as i64;
    let v_vec = l1b - l1a;
    let h_vec = l2b - l2a;
    let aabb = q.xyxy();
    let Some((x1, y1, cw, ch)) = clamp_crop(aabb.0, aabb.1, aabb.2, aabb.3, im_w, im_h) else {
        return Ok(None);
    };
    let ratio = v_vec.norm() / h_vec.norm();

    // cv2.warpPerspective could overflow if image size is too large, better crop it here
    let img_croped = RawImage::from(DynamicImage::from(
        image::imageops::crop_imm(img, x1, y1, cw, ch).to_image(),
    ));

    let img_croped = img_croped.as_opencv_mat()?;
    let src_points = q
        .pts()
        .iter()
        .map(|v| Point2f::new((v.x - x1 as i64) as f32, (v.y - y1 as i64) as f32))
        .collect::<Vector<Point2f>>();

    let h = text_height.max(2);
    let (w, h) = if q.vertical() {
        let w = ((text_height as f64 * ratio).round() as u32).max(2);
        (h, w)
    } else {
        let w = ((text_height as f64 / ratio).round() as u32).max(2);
        (w, h)
    };
    let dst_points = [
        Point2f::new(0.0, 0.0),
        Point2f::new(w as f32 - 1.0, 0.0),
        Point2f::new(w as f32 - 1.0, h as f32 - 1.0),
        Point2f::new(0.0, h as f32 - 1.0),
    ]
    .into_iter()
    .collect::<Vector<Point2f>>();
    let m = find_homography(&src_points, &dst_points, &mut no_array(), RANSAC, 5.0)?;
    let mut region = Mat::default();
    opencv::imgproc::warp_perspective(
        &img_croped,
        &mut region,
        &m,
        Size::new(w as i32, h as i32),
        INTER_LINEAR,
        BORDER_CONSTANT,
        Scalar::default(),
    )?;
    if q.vertical() {
        let mut reg = Mat::default();
        rotate(&region, &mut reg, ROTATE_90_COUNTERCLOCKWISE)?;
        region = reg;
    }

    Ok(Some(region))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::RgbImage;
    use interface_detector::textlines::Quadrilateral;

    #[test]
    fn inverted_edges_do_not_overflow_crop() {
        let (x, y, w, h) = clamp_crop(0, 5, 4, 2, 8, 6).unwrap_or((0, 0, 0, 0));
        assert!(u64::from(x) + u64::from(w) <= 8);
        assert!(u64::from(y) + u64::from(h) <= 6);
        assert!(clamp_crop(0, 5, 4, 2, 8, 6).is_none());
    }

    #[test]
    fn oob_quad_does_not_panic() {
        let img = RgbImage::new(20, 10);
        let q = Quadrilateral::new(vec![(-8, -4), (30, -2), (28, 18), (-6, 16)], 1.0);
        let _ = get_transformed_region(&q, &img, 48);
    }
}
