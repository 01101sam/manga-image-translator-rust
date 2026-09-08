use imageproc::{
    distance_transform::Norm,
    image::{DynamicImage, GenericImageView, GrayImage},
    morphology::erode,
};

use interface_detector::textlines::Quadrilateral;
use interface_image::{Mask, MaskCow, RawImage, RawImageCow};
use ndarray::{s, Array2, ArrayView2, Zip};
use opencv::{
    core::{
        bitwise_or, in_range, subtract, Mat, MatExprTraitConst, MatTraitConst,
        MatTraitConstManual as _, MatTraitManual as _, Point, Rect, Scalar, Size, BORDER_CONSTANT,
        CV_16U, CV_8U,
    },
    imgproc::{get_structuring_element, MORPH_ELLIPSE, MORPH_RECT, THRESH_BINARY, THRESH_OTSU},
};
use rayon::prelude::*;
use roots::find_roots_quadratic;

pub fn refine_mask(
    img: &RawImageCow,
    mask: Mask,
    blk_list: &[Quadrilateral],
    refinemask_inpaint: bool,
) -> anyhow::Result<Mask> {
    let img = img.view();
    let img_ = img.to_image()?;
    // Blocks are independent; only the final OR into the shared mask needs to be sequential.
    let refined_blocks = blk_list
        .par_iter()
        .map(|blk| {
            let (bx1, by1, bx2, by2) =
                enlarge_window(blk.xyxy(), img.width, img.height, 2.5, 1.0);
            let roi_rect = Rect::new(
                bx1 as i32,
                by1 as i32,
                (bx2 - bx1) as i32,
                (by2 - by1) as i32,
            );
            let im = DynamicImage::from(
                img_.view(
                    bx1 as u32,
                    by1 as u32,
                    (bx2 - bx1) as u32,
                    (by2 - by1) as u32,
                )
                .to_image(),
            );
            let im_gray = Mask::from(im.clone().into_luma8());
            let im_gray = im_gray.as_nd()?;
            let msk = mask.as_nd()?;
            let msk = msk.slice(s![by1 as usize..by2 as usize, bx1 as usize..bx2 as usize]);
            let mut mask_list = get_topk_masklist(im_gray, &msk)?;
            mask_list.extend(get_otsuthresh_masklist(&RawImage::from(im), msk)?);
            let mask_merged =
                merge_mask_list(mask_list, &MaskCow::from(msk), 30, refinemask_inpaint)?;
            Ok((roi_rect, mask_merged))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    let mut mask_refined = Mat::zeros(mask.height as i32, mask.width as i32, CV_8U)?.to_mat()?;
    for (roi_rect, mask_merged) in refined_blocks {
        let mut roi = Mat::roi_mut(&mut mask_refined, roi_rect)?;
        let roy2 = roi.clone_pointee();
        bitwise_or(&roy2, &mask_merged, &mut roi, &opencv::core::no_array())?;
    }
    Ok(Mask::from(mask_refined))
}

fn ndarray_to_gray_image(arr: &ArrayView2<u8>) -> GrayImage {
    let data = arr
        .as_slice()
        .map(|v| v.to_vec())
        .unwrap_or_else(|| arr.iter().cloned().collect());
    GrayImage::from_raw(arr.dim().1 as u32, arr.dim().0 as u32, data).unwrap()
}

fn gray_image_to_ndarray(img: &GrayImage) -> anyhow::Result<ArrayView2<'_, u8>> {
    let (width, height) = img.dimensions();
    let data = img.as_raw();
    Ok(ArrayView2::from_shape(
        (height as usize, width as usize),
        data,
    )?)
}

fn extract_candidates(im_grey: &ArrayView2<u8>, mask: &ArrayView2<u8>) -> anyhow::Result<Vec<u8>> {
    let mask_img = ndarray_to_gray_image(mask);
    let eroded_img = erode(&mask_img, Norm::LInf, 1);
    let eroded_mask = gray_image_to_ndarray(&eroded_img)?;
    let mut result = Vec::new();
    for ((y, x), &val) in eroded_mask.indexed_iter() {
        if val > 127 {
            result.push(im_grey[(y, x)]);
        }
    }
    Ok(result)
}

fn get_topk_masklist(
    im_grey: ArrayView2<u8>,
    ped_mask: &ArrayView2<u8>,
) -> anyhow::Result<Vec<(Array2<u8>, u64)>> {
    let candidate_grey_px = extract_candidates(&im_grey, &ped_mask)?;
    let (bin, his) = histogram(&candidate_grey_px, 255);
    let topk_color = get_topk_color(his, bin, 3, 10, 0.001);
    let color_range = 30;
    topk_color
        .into_iter()
        .map(|color| {
            let c_top = 255.min(color + color_range);
            let c_bottom = c_top as i64 - 2 * color_range as i64;
            let mut threshed = Mat::default();
            let im_grey = MaskCow::from(im_grey.view());
            let im_grey = im_grey.view().as_opencv_mat()?;

            in_range(
                &im_grey,
                &Scalar::all(c_bottom as f64),
                &Scalar::all(c_top as f64),
                &mut threshed,
            )?;
            let threshed = Mask::from(threshed);
            let threshed = threshed.as_nd()?;

            let (threshed, xor_sum) = minxor_thresh(threshed, &ped_mask, false);
            Ok((threshed, xor_sum))
        })
        .collect()
}

fn argsort_descending(v: &[i32]) -> Vec<usize> {
    let mut indices: Vec<usize> = (0..v.len()).collect();
    indices.sort_by_key(|&i| -v[i]);
    indices
}

fn get_topk_color(
    color_list: Vec<u32>,
    bins: Vec<usize>,
    k: usize,
    color_var: u32,
    bin_tol: f64,
) -> Vec<u32> {
    let color_list_ = color_list;
    let color_list = bins;
    let bins = color_list_
        .into_iter()
        .map(|v| v as usize)
        .collect::<Vec<_>>();
    let idx = argsort_descending(&bins.iter().map(|v| *v as i32).collect::<Vec<_>>());
    let mut color_list = idx.iter().map(|v| color_list[*v]);
    let bins = idx.iter().map(|v| bins[*v]).collect::<Vec<_>>();
    let mut top_colors = vec![color_list.next().unwrap()];
    let bin_tol = bins.iter().sum::<usize>() as f64 * bin_tol;
    for (color, bin) in color_list.zip(bins.iter()) {
        if let Some(v) = top_colors.iter().map(|v| (*v) as i32 - color as i32).min() {
            let v = if v < 0 { (v * -1) as u32 } else { v as u32 };
            if v > color_var {
                top_colors.push(color);
            }
        }
        if top_colors.len() >= k || (*bin as f64) < bin_tol {
            break;
        }
    }
    top_colors.into_iter().map(|v| v as u32).collect()
}

fn histogram(candidate_grey_px: &[u8], bins: usize) -> (Vec<usize>, Vec<u32>) {
    assert!(bins <= 256, "bins should be ≤ 256 for u8 input");

    let mut hist = vec![0u32; bins];
    for &val in candidate_grey_px {
        let bin = (val as usize * bins) / 256;
        if bin < bins {
            hist[bin] += 1;
        }
    }

    let bin_edges: Vec<usize> = (0..=bins).map(|b| (b * 256) / bins).collect();

    (bin_edges, hist)
}

/// Adds one connected component (`stat` = x, y, w, h of its bounding box) to `mask_merged` when
/// that lowers `sum(mask_merged ^ pred_mask)` over the box. Pixels outside the component are
/// identical in both sums, so only the component's pixels decide.
fn merge_component(
    stat: &[i32],
    labels: &Mat,
    label_index: u16,
    mask_merged: &mut Mat,
    pred_mask: &Mat,
) -> opencv::Result<()> {
    let cols = labels.cols() as usize;
    let rows = stat[1] as usize..(stat[1] + stat[3]) as usize;
    let span = |row: usize| row * cols + stat[0] as usize..row * cols + (stat[0] + stat[2]) as usize;
    let labels = labels.data_typed::<u16>()?;
    let pred = pred_mask.data_typed::<u8>()?;
    let merged = mask_merged.data_typed_mut::<u8>()?;

    let mut delta = 0i32;
    for row in rows.clone() {
        for i in span(row) {
            if labels[i] == label_index {
                delta += (255 ^ pred[i]) as i32 - (merged[i] ^ pred[i]) as i32;
            }
        }
    }
    if delta < 0 {
        for row in rows {
            for i in span(row) {
                if labels[i] == label_index {
                    merged[i] = 255;
                }
            }
        }
    }
    Ok(())
}

fn get_area_threshold(stats: &Mat) -> opencv::Result<i32> {
    let rows = stats.rows();
    let mut areas = Vec::with_capacity(rows as usize);

    for i in 0..rows {
        let area = *stats.at_2d::<i32>(i, 4)?;
        areas.push(area);
    }

    areas.sort_unstable();

    let area_thresh = if areas.len() > 1 {
        areas[areas.len() - 2]
    } else {
        areas[0]
    };

    Ok(area_thresh)
}

fn merge_mask_list(
    mut mask_list: Vec<(Array2<u8>, u64)>,
    pred_mask: &MaskCow,
    pred_thresh: u32,
    refinemask_inpaint: bool,
) -> anyhow::Result<Mat> {
    mask_list.sort_by_key(|v| v.1);

    let pred_mask = if pred_thresh > 0 {
        let e_size = 1;
        let element = opencv::imgproc::get_structuring_element(
            MORPH_ELLIPSE,
            Size::new(2 * e_size + 1, 2 * e_size + 1),
            Point::new(e_size, e_size),
        )?;
        let pred_mask = pred_mask.view().as_opencv_mat()?;
        let mut pred_mask_out =
            Mat::zeros(pred_mask.rows(), pred_mask.cols(), pred_mask.typ())?.to_mat()?;
        opencv::imgproc::erode(
            &pred_mask,
            &mut pred_mask_out,
            &element,
            Point::new(-1, -1),
            1,
            BORDER_CONSTANT,
            Scalar::all(0.0),
        )?;
        let mut pred_mask =
            Mat::zeros(pred_mask.rows(), pred_mask.cols(), pred_mask.typ())?.to_mat()?;
        opencv::imgproc::threshold(
            &pred_mask_out,
            &mut pred_mask,
            60 as f64,
            255 as f64,
            THRESH_BINARY,
        )?;
        pred_mask
    } else {
        todo!()
    };

    let size = pred_mask.size()?;
    let typ = pred_mask.typ();
    let mut mask_merged = Mat::zeros_size(size, typ)?.to_mat()?;

    let connectivity = 8;
    for (candidate_mask, _) in mask_list.into_iter() {
        let mut labels = Mat::default();
        let mut stats = Mat::default();
        let mut centroids = Mat::default();
        let candidate_mask = Mask::from(candidate_mask);
        let candidate_mask = candidate_mask.as_opencv_mat()?;
        let num_labels = opencv::imgproc::connected_components_with_stats(
            &candidate_mask,
            &mut labels,
            &mut stats,
            &mut centroids,
            connectivity,
            CV_16U,
        )?;
        for label_index in 1..num_labels {
            let stat = stats.at_row::<i32>(label_index)?;
            if stat[2] * stat[3] < 3 {
                continue;
            }
            merge_component(stat, &labels, label_index as u16, &mut mask_merged, &pred_mask)?;
        }
        if refinemask_inpaint {
            let kernel = get_structuring_element(
                MORPH_RECT,
                Size::new(5, 5),
                opencv::core::Point::new(-1, -1),
            )?;
            let mut dst = Mat::default();
            opencv::imgproc::dilate(
                &mask_merged,
                &mut dst,
                &kernel,
                opencv::core::Point::new(-1, -1),
                1,
                opencv::core::BORDER_CONSTANT,
                Scalar::default(),
            )?;
            mask_merged = dst;
        }
        let mut labels = Mat::default();
        let mut stats = Mat::default();
        let mut centroids = Mat::default();
        let mut inverted = Mat::default();
        let scalar_255 = Scalar::all(255.0);

        subtract(
            &scalar_255,
            &mask_merged,
            &mut inverted,
            &opencv::core::no_array(),
            -1,
        )?;
        let num_labels = opencv::imgproc::connected_components_with_stats(
            &inverted,
            &mut labels,
            &mut stats,
            &mut centroids,
            connectivity,
            CV_16U,
        )?;
        let area_thresh = get_area_threshold(&stats)?;
        for label_index in 0..num_labels {
            let stat = stats.at_row::<i32>(label_index)?;
            if stat[4] < area_thresh {
                merge_component(stat, &labels, label_index as u16, &mut mask_merged, &pred_mask)?;
            }
        }
    }
    Ok(mask_merged)
}

fn get_otsuthresh_masklist(
    img: &RawImage,
    pred_mask: ArrayView2<u8>,
) -> anyhow::Result<Vec<(Array2<u8>, u64)>> {
    let channels = img.channels();
    let h = img.height;
    let mask_list = channels
        .into_iter()
        .map(|c| {
            let mut threshed = Mat::default();
            let c = Mat::from_slice(&c)?;
            let c = c.reshape(1, h as i32)?;
            opencv::imgproc::threshold(&c, &mut threshed, 1.0, 255.0, THRESH_OTSU | THRESH_BINARY)?;
            let threshed = Mask::from(threshed);
            let threshed = threshed.as_nd()?;
            let (threshed, xor_sum) = minxor_thresh(threshed, &pred_mask, false);
            Ok((threshed, xor_sum))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(vec![mask_list.into_iter().min_by_key(|v| v.1).unwrap()])
}

fn minxor_thresh(
    threshed: ArrayView2<u8>,
    mask: &ArrayView2<u8>,
    dilate: bool,
) -> (Array2<u8>, u64) {
    let neg_threshed = threshed.mapv(|v| 255 - v);
    if dilate {
        // let e_size = 1;
        // element = cv2.getStructuringElement(cv2.MORPH_RECT, (2 * e_size + 1, 2 * e_size + 1),(e_size, e_size))
        // neg_threshed = cv2.dilate(neg_threshed, element, iterations=1)
        // threshed = cv2.dilate(threshed, element, iterations=1)
        unimplemented!()
    }

    let neg_xor_sum = Zip::from(&neg_threshed)
        .and(mask)
        .fold(0u64, |acc, &x, &y| acc + (x ^ y) as u64);
    let xor_sum = Zip::from(&threshed)
        .and(mask)
        .fold(0u64, |acc, &x, &y| acc + (x ^ y) as u64);

    if neg_xor_sum < xor_sum {
        return (neg_threshed, neg_xor_sum);
    } else {
        return (threshed.to_owned(), xor_sum);
    }
}

fn enlarge_window(
    (x1, y1, x2, y2): (i64, i64, i64, i64),
    im_w: u16,
    im_h: u16,
    ratio: f32,
    aspect_ratio: f32,
) -> (i64, i64, i64, i64) {
    assert!(ratio > 1.0);
    let w = x2 - x1;
    let h = y2 - y1;

    let coeff_a = aspect_ratio;
    let coeff_b = w as f32 + h as f32 * aspect_ratio;
    let coeff_c = (1.0 - ratio) * w as f32 * h as f32;

    let roots = find_roots_quadratic(coeff_a, coeff_b, coeff_c);

    let valid_root = match roots {
        roots::Roots::No(_) => None,
        roots::Roots::One(one) => one.into_iter().find(|r| r.is_finite() && *r > 0.0),
        roots::Roots::Two(two) => two
            .iter()
            .copied()
            .filter(|r| r.is_finite() && *r > 0.0)
            .max_by(|a, b| a.partial_cmp(b).unwrap()),
        roots::Roots::Three(t) => t
            .iter()
            .copied()
            .filter(|r| r.is_finite() && *r > 0.0)
            .max_by(|a, b| a.partial_cmp(b).unwrap()),
        roots::Roots::Four(t) => t
            .iter()
            .copied()
            .filter(|r| r.is_finite() && *r > 0.0)
            .max_by(|a, b| a.partial_cmp(b).unwrap()),
    };

    let max = valid_root.expect("No valid root found");
    let delta = (max / 2.0).round() as i64;
    let delta_w = (delta as f32 * aspect_ratio).round() as i64;

    let delta_w = *[x1, im_w as i64 - x2, delta_w].iter().min().unwrap();
    let delta = *[y1, im_h as i64 - y2, delta].iter().min().unwrap();

    (x1 - delta_w, y1 - delta, x2 + delta_w, y2 + delta)
}
