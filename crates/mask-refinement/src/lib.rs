mod bubble;
mod expand;
mod fill_text;

pub use bubble::is_ignore;

use std::{borrow::Cow, i32, sync::Arc};

use interface_detector::textlines::{BBox, MyPoint, Quadrilateral};
use interface_image::{ImageOp, Mask, RawImage};
use opencv::{
    core::{
        bitwise_and, no_array, Mat, MatTraitConst, Point, Scalar, Vector, BORDER_CONSTANT, CV_8UC1,
    },
    imgproc::{
        bounding_rect, dilate, draw_contours, find_contours, morphology_default_border_value,
        rectangle, CHAIN_APPROX_SIMPLE, LINE_8, RETR_EXTERNAL,
    },
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use textline_merge::TextBlock;

use crate::{
    expand::{
        expand_right_quad, expand_right_to_connect, expand_top_quad, expand_top_to_connect,
        shrink_quad_right, shrink_quad_top,
    },
    fill_text::complete_mask,
};

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Method {
    FitText,
    FillMask,
}

pub fn expand(furi: bool, lines: &[[MyPoint; 4]], mask: &Mask) -> Vec<Quadrilateral> {
    let mut out = Vec::with_capacity(lines.len());
    let mut lines = lines.iter().rev().peekable();
    while let Some(line) = lines.next() {
        let line_ = Quadrilateral::new2(line.to_vec(), 0.0);
        if furi {
            let peek = lines.peek();
            match line_.vertical() {
                true => out.push(Quadrilateral::new2(
                    match peek {
                        Some(n) => expand_right_to_connect(line, n),
                        None => shrink_quad_right(expand_right_quad(*line, 2.0), mask),
                    }
                    .to_vec(),
                    0.0,
                )),
                false => out.push(Quadrilateral::new2(
                    match peek {
                        Some(n) => expand_top_to_connect(line, n),
                        None => shrink_quad_top(expand_top_quad(*line, 2.0), mask),
                    }
                    .to_vec(),
                    0.0,
                )),
            }
        } else {
            out.push(line_);
        }
    }
    out
}
pub fn dispatch(
    text_regions: &[TextBlock],
    raw_img: &RawImage,
    raw_mask: &Mask,
    method: Method,
    ignore_bubble: u8,
    dilation_offset: f64,
    kernel_size: i32,
    furi: bool,
    image_op: &Arc<dyn ImageOp + Send + Sync>,
) -> anyhow::Result<Mask> {
    let raw_mask_ = if furi {
        Cow::Owned(image_op.resize_mask(
            raw_mask.view(),
            raw_img.width as usize,
            raw_img.height as usize,
            interface_image::Interpolation::Nearest,
        )?)
    } else {
        Cow::Borrowed(raw_mask)
    };
    let size = (raw_img.width, raw_img.height);

    let textlines = text_regions
        .iter()
        .flat_map(|v| expand(furi, &v.lines, raw_mask_.as_ref()))
        .collect::<Vec<_>>();

    let final_mask = match method {
        Method::FitText => {
            let scale_factor = ((raw_mask.height as f64 - raw_img.height as f64 / 3.0)
                / raw_mask.height as f64)
                .clamp(0.5, 1.0);
            let n_w = (raw_img.width as f64 * scale_factor) as u16;
            let n_h = (raw_img.height as f64 * scale_factor) as u16;
            let mut img_resized = image_op.resize(
                raw_img.view(),
                n_w,
                n_h,
                interface_image::Interpolation::Nearest,
            )?;
            let img_resized = img_resized.as_opencv_mut_mat()?;
            let mask_resized = image_op.resize_mask(
                raw_mask.view(),
                n_w as usize,
                n_h as usize,
                interface_image::Interpolation::Nearest,
            )?;
            let mut mask_resized =
                Mask::from(mask_resized.as_nd()?.mapv(|v| if v > 0 { 255 } else { 0 }));
            let mask_resized = mask_resized.as_opencv_mut_mat()?;

            let final_mask = complete_mask(
                textlines
                    .into_iter()
                    .map(|v| v.scale(scale_factor))
                    .collect(),
                img_resized,
                mask_resized,
                1e-2,
                dilation_offset,
                kernel_size,
            )?;
            match final_mask {
                Some(mask) => {
                    let mask = Mask::from(mask);
                    let mut mask = image_op
                        .resize_mask(
                            mask.view(),
                            size.0 as usize,
                            size.1 as usize,
                            interface_image::Interpolation::Nearest,
                        )?
                        .to_nd()?;
                    mask.mapv_inplace(|v| if v > 0 { 255 } else { 0 });
                    let mask = Mask::from(mask);
                    mask.data
                }
                None => vec![0; size.0 as usize * size.1 as usize],
            }
        }
        Method::FillMask => complete_mask_fill(
            (size.0 as i64, size.1 as i64),
            textlines.iter().map(|v| v.aabb()).collect::<Vec<_>>(),
        ),
    };

    if ignore_bubble < 1 || ignore_bubble > 50 {
        return Ok(Mask {
            width: size.0,
            height: size.1,
            data: final_mask,
        });
    }
    let kernel_size = (size.0.max(size.1) as f64 * 0.025) as usize;
    let ones = vec![1_u8; kernel_size * kernel_size];
    let kernel = Mat::from_slice(&ones)?;
    let kernel = kernel.reshape(1, kernel_size as i32)?;
    let final_mask = Mat::from_slice(&final_mask)?;
    let final_mask = final_mask.reshape(1, size.1 as i32)?;
    let mut new_final_mask = Mat::default();
    dilate(
        &final_mask,
        &mut new_final_mask,
        &kernel,
        Point::new(-1, -1),
        1,
        BORDER_CONSTANT,
        morphology_default_border_value()?,
    )?;
    let mut final_mask = new_final_mask;

    let mut contours = Vector::<Vector<Point>>::new();

    find_contours(
        &final_mask,
        &mut contours,
        RETR_EXTERNAL,
        CHAIN_APPROX_SIMPLE,
        Point::default(),
    )?;
    for cnt in contours {
        let mut temp_mask = Mat::new_rows_cols_with_default(
            size.1 as i32,
            size.0 as i32,
            CV_8UC1,
            Scalar::all(0.0),
        )?;
        // # rect min
        let rec = bounding_rect(&cnt)?;
        rectangle(&mut temp_mask, rec, Scalar::all(255.0), -1, LINE_8, 0)?;
        // get textblock
        let raw_image = raw_img.as_opencv_mat()?;
        let mut textblock = Mat::default();
        bitwise_and(&raw_image, &raw_image, &mut textblock, &temp_mask)?;
        if is_ignore(size, &textblock, ignore_bubble)? {
            draw_contours(
                &mut final_mask,
                &vec![cnt].into_iter().collect::<Vector<Vector<Point>>>(),
                -1,
                Scalar::all(0.0),
                -1,
                LINE_8,
                &no_array(),
                i32::MAX,
                Point::default(),
            )?;
        }
    }
    Ok(Mask::from(final_mask))
}

fn complete_mask_fill(size: (i64, i64), aabbs: Vec<BBox>) -> Vec<u8> {
    let (width, height) = size;
    let mut mask = vec![0u8; (width * height) as usize];

    for aabb in aabbs {
        let x0 = aabb.x.min(width);
        let y0 = aabb.y.min(height);
        let x1 = (aabb.w + aabb.x).min(width);
        let y1 = (aabb.h + aabb.y).min(height);

        for y in y0..y1 {
            let row_start = (y * width) as usize;
            for x in x0..x1 {
                mask[row_start + x as usize] = 255;
            }
        }
    }

    mask
}
