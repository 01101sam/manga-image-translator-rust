use std::ops::Range;

use anyhow::anyhow;
use base_util::opencv_utils::to_continous2;
use ndarray::{Array3, ArrayView3, Axis};
use opencv::{
    core::{Mat, MatTraitConst as _, MatTraitConstManual, CV_8U, CV_MAT_DEPTH},
    imgproc::{threshold, THRESH_BINARY},
};

///     Determine whether there are colors in non-black, gray, white, and other gray areas in an RGB color image.
///     params：
///     image -- np.array
///     return：
///     True -- Colors with non black, gray, white, and other grayscale areas
///     False -- Images are all grayscale areas
fn check_color(image: &Mat) -> anyhow::Result<bool> {
    let image = convert(&image)?;
    let weights = [0.299_f32, 0.587_f32, 0.114_f32];
    // Calculate grayscale version of the image using vectorized operations
    let gray_image = image.map_axis(Axis(2), |rgb| {
        rgb[0] * weights[0] + rgb[1] * weights[1] + rgb[2] * weights[2]
    });

    let gray_image = gray_image.insert_axis(Axis(2));

    // Calculate color distance for all pixels in a vectorized manner
    let diff = &image
        - &gray_image
            .broadcast(image.dim())
            .ok_or(anyhow!("failed to broadcast gray_image"))?;

    // Square each difference
    let sq = diff.map(|x| x.powi(2));

    // Sum along channel axis (-1 in numpy → Axis(2) here)
    let color_distance = sq.sum_axis(Axis(2));
    // Count the number of pixels where color distance exceeds the threshold
    let n = color_distance.iter().filter(|&&x| x > 100.0).count();
    // Return True if there are more than 10 such pixels
    // TODO:
    // Proportion should be used
    Ok(n > 10)
}

fn convert(mat: &Mat) -> Result<Array3<f32>, ndarray::ShapeError> {
    let mat_type = mat.typ();
    assert_eq!(CV_MAT_DEPTH(mat_type), CV_8U, "Expected CV_8U Mat");
    assert_eq!(mat.channels(), 3, "Expected single-channel grayscale Mat");
    let mat = to_continous2(mat);
    let data = mat.data_bytes().expect("to_continous used");
    let rows = mat.rows() as usize;
    let cols = mat.cols() as usize;
    Ok(ArrayView3::from_shape((rows, cols, 3), data)?.mapv(|x| x as f32))
}

fn count_zero_pixels_in_range(
    mat: &Mat,
    row_range: Range<i32>,
    col_range: Range<i32>,
) -> anyhow::Result<(i32, i32)> {
    let roi = Mat::roi(
        mat,
        opencv::core::Rect::new(
            col_range.start,                 // x
            row_range.start,                 // y
            col_range.end - col_range.start, // width
            row_range.end - row_range.start, // height
        ),
    )?;
    if !roi.is_continuous() {
        let mut continuous_mat = Mat::default();
        roi.copy_to(&mut continuous_mat)?;
        let roi_u8 = continuous_mat
            .data_bytes()
            .expect("Copied so is continuous");
        let zero_count = roi_u8.iter().filter(|&&v| v == 0).count() as i32;
        let total_count = roi_u8.len() as i32;
        Ok((zero_count, total_count))
    } else {
        let roi_u8 = roi.data_bytes().expect("checked to be continuous");
        let zero_count = roi_u8.iter().filter(|&&v| v == 0).count() as i32;
        let total_count = roi_u8.len() as i32;
        Ok((zero_count, total_count))
    }
}

///     Principle: Normally, white bubbles and their text boxes are mostly white, while black bubbles and their text boxes are mostly black. We calculate the ratio of white or black pixels around the text block to the total pixels, and judge whether the area is a normal bubble area or not. Based on the value of the --ignore-bubble parameter, if the ratio is greater than the base value and less than (100-base value), then it is considered a non-bubble area.
///     The normal range for ignore-bubble is 1-50, and other values are considered not input. The recommended value for ignore-bubble is 10. The smaller it is, the more likely it is to recognize normal bubbles as image text and skip them. The larger it is, the more likely it is to recognize image text as normal bubbles.
///
///     Assuming ignore-bubble = 10
///     The text block is surrounded by white if it is <10, and the text block is very likely to be a normal white bubble.
///     The text block is surrounded by black if it is >90, and the text block is very likely to be a normal black bubble.
///     Between 10 and 90, if there are black and white spots around it, the text block is very likely not a normal bubble, but an image.
///
///     The input parameter is the image data of the text block processed by OCR.
///     Calculate the ratio of black or white pixels in the four rectangular areas formed by taking 2 pixels from the edges of the four sides of the image.
///     Return the overall ratio. If it is between ignore_bubble and (100-ignore_bubble), skip it.
///
///     last determine if there is color, consider the colored text as invalid information and skip it without translation
///
///     # Current issues with bubble detection:
///     # 1. Misjudgment of solid color backgrounds (core issue):
///     # Reason: The code calculates the black/white pixel ratio in a 2-pixel edge area around the text box. If the text box is on a large solid white background (e.g., black text on white paper), the edges will mostly be white, resulting in a very low ratio (close to 0), which falls below the ignore_bubble threshold. The code then mistakenly considers this as a "normal white bubble background" and fails to ignore it (i.e., it treats it as regular bubble text that needs translation). While this text does require translation, it is not actually bubble text.
///     # Fundamental flaw: This method does not detect bubble boundaries or contours; it only checks local background color.
///     # 2. Inability to recognize bubble boundaries:
///     # Reason: The code does not involve any shape or contour detection. It cannot determine whether there is a closed, relatively uniform-colored line surrounding the text box.
///     # Consequence: Unable to distinguish between actual bubbles (with boundaries) and cases where the background color coincidentally meets the ratio criteria.
///     # 3. Insensitivity to bubble size and relative position:
///     # Reason: Only examines the immediate 2-pixel area, without considering the overall size, shape of the bubble, or the text box's relative position within the bubble.
///     # Consequence: Cannot utilize common-sense features like "bubbles typically surround the text box and are moderately sized."
///     # 4. Connected bubble issue:
///     # Reason: The current logic is entirely based on the local environment of a single text box and cannot detect whether there is a shared bubble structure spanning multiple text boxes.
///     # Consequence: Unable to handle cases where a large or complex-shaped bubble contains multiple independent text blocks, nor can it determine which part of the bubble corresponds to which text block.
pub fn is_ignore(
    (width, height): (u16, u16),
    region_img: &Mat,
    ignore_bubble: u8,
) -> anyhow::Result<bool> {
    if ignore_bubble < 1 || ignore_bubble > 50 {
        return Ok(false);
    }

    let mut binary_raw_mask = Mat::default();
    threshold(
        region_img,
        &mut binary_raw_mask,
        127.0,
        255.0,
        THRESH_BINARY,
    )?;

    let mut total = 0;
    let mut val0 = 0;
    let (zeros, count) = count_zero_pixels_in_range(&binary_raw_mask, 0..2, 0..width as i32)?;
    val0 += zeros;
    total += count;

    let (zeros, count) = count_zero_pixels_in_range(
        &binary_raw_mask,
        (height as i32 - 2)..height as i32,
        0..width as i32,
    )?;
    val0 += zeros;
    total += count;

    let (zeros, count) = count_zero_pixels_in_range(&binary_raw_mask, 2..height as i32 - 2, 0..2)?;
    val0 += zeros;
    total += count;

    let (zeros, count) = count_zero_pixels_in_range(
        &binary_raw_mask,
        2..height as i32 - 2,
        width as i32 - 2..width as i32,
    )?;
    val0 += zeros;
    total += count;

    let ratio = round_to_places(val0 as f64 / total as f64, 6) * 100.0;
    // ignore
    if ratio >= ignore_bubble as f64 && ratio <= (100 - ignore_bubble) as f64 {
        return Ok(true);
    }
    // To determine if there is color, consider the colored text as invalid information and skip it without translation
    Ok(if check_color(region_img)? {
        true
    } else {
        false
    })
}

fn round_to_places(value: f64, places: u32) -> f64 {
    let factor = 10f64.powi(places as i32);
    (value * factor).round() / factor
}
